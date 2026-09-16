//! CEF's page-scoped protocol implements the shared semantic driver's ports.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use nomifun_browser_macos::{engine::Page, protocol::CallbackSubscription};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(crate) struct View {
    pub page: Arc<Page>,
    pub diagnostics_failed: Arc<AtomicBool>,
}
impl View {
    pub fn new(page: Arc<Page>) -> Self { Self { page, diagnostics_failed: Arc::new(AtomicBool::new(false)) } }
}

#[path = "../frame_sessions.rs"]
pub(crate) mod frames;
#[path = "native/file_chooser.rs"]
pub(crate) mod file_chooser;

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
    pub async fn enable_session(view: &View, session: Option<&str>) -> Result<(), String> {
        for method in ["Runtime.enable", "Log.enable", "Network.enable"] {
            protocol_call_session(view, session, method, serde_json::json!({})).await?;
        }
        Ok(())
    }
    pub fn mark_unavailable(view: &View) { view.diagnostics_failed.store(true, Ordering::Release); }
}

pub(crate) async fn set_user_input_enabled(view: &View, enabled: bool) -> Result<(), String> {
    if enabled {
        protocol_call(view, "Page.setInterceptFileChooserDialog", serde_json::json!({"enabled":false})).await?;
        return set_native_user_input_enabled(view, true).await;
    }
    let input = set_native_user_input_enabled(view, false).await;
    // Renderer commands can wait on a user dialog. Freeze native input first,
    // then drain that dialog before awaiting root chooser interception.
    script_dialogs::drain(view).await.map_err(|error| error.to_string())?;
    let chooser = protocol_call(view, "Page.setInterceptFileChooserDialog", serde_json::json!({"enabled":true})).await.map(|_|());
    input.and(chooser)
}
