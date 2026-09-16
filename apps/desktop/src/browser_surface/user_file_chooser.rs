//! UserReady HTML chooser broker. The native event supplies the target, never
//! renderer IPC or model JSON. Every result belongs to one document and request.
use super::super::automation::TabAutomation;
use super::{
    file_chooser::{Choice, from_event},
    frames::OwnedFrameRoute,
    user_file_picker::{NativeFilePicker, Options, PickerMode},
};
use serde_json::json;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio_util::sync::CancellationToken;
use webview2_com::{DevToolsProtocolEventReceivedEventHandler, Microsoft::Web::WebView2::Win32::*};
use windows::core::{HSTRING, PCWSTR, PWSTR};

struct Pending {
    cancel: CancellationToken,
    done: watch::Sender<Option<Result<(), String>>>,
    picker: AsyncMutex<Option<NativeFilePicker>>,
    worker_failed: AtomicBool,
    frames: Mutex<Option<Vec<String>>>,
}
impl Pending {
    async fn cleanup(&self) -> Result<(), String> {
        let mut picker = self.picker.lock().await;
        if let Some(picker) = picker.as_ref() {
            picker.close().await?;
        }
        picker.take();
        Ok(())
    }
    async fn close(&self) -> Result<(), String> {
        self.cancel.cancel();
        let mut done = self.done.subscribe();
        loop {
            if done.borrow().is_some() {
                break;
            }
            done.changed()
                .await
                .map_err(|_| "File request completion was lost")?;
        }
        self.cleanup().await?;
        if self.worker_failed.load(Ordering::Acquire) {
            return Err("File request worker did not settle normally".into());
        }
        self.done.send_replace(Some(Ok(())));
        Ok(())
    }
}
struct Control {
    visible: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    locked: Arc<AtomicBool>,
    pending: Mutex<Option<Arc<Pending>>>,
}
impl Control {
    fn document_changed(&self, frame: Option<&str>) {
        if let Some(pending) = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            let frames = pending
                .frames
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if frame.is_none()
                || frames
                    .as_ref()
                    .is_none_or(|frames| frames.iter().any(|id| Some(id.as_str()) == frame))
            {
                pending.cancel.cancel();
            }
        }
    }
    fn cancel(&self) {
        if let Some(pending) = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            pending.cancel.cancel();
        }
    }
    fn permitted(&self) -> bool {
        self.visible.load(Ordering::Acquire)
            && !self.closed.load(Ordering::Acquire)
            && !self.locked.load(Ordering::Acquire)
    }
}
struct Registration {
    control: Arc<Control>,
    events: Vec<(ICoreWebView2DevToolsProtocolEventReceiver, i64)>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.control.closed.store(true, Ordering::Release);
        self.control.cancel();
        for (event, token) in &self.events {
            let _ = unsafe { event.remove_DevToolsProtocolEventReceived(*token) };
        }
    }
}
thread_local! {static REGISTRATIONS:RefCell<HashMap<String,Registration>>=RefCell::default();}
pub(super) fn installed(label: &str) -> bool {
    REGISTRATIONS.with(|entries| entries.borrow().contains_key(label))
}
pub(crate) fn set_visible(label: &str, visible: bool) {
    REGISTRATIONS.with(|entries| {
        if let Some(entry) = entries.borrow().get(label) {
            entry.control.visible.store(visible, Ordering::Release);
            if !visible {
                entry.control.cancel();
            }
        }
    });
}
pub(crate) fn cancel(label: &str) {
    REGISTRATIONS.with(|entries| {
        if let Some(entry) = entries.borrow().get(label) {
            entry.control.cancel();
        }
    });
}
pub(super) fn close_view(label: &str) {
    REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label));
}

/// Read-only native conformance probe; never exposed through IPC or a Tool.
pub(crate) async fn active_picker(
    view: &tauri::Webview,
) -> Result<Option<NativeFilePicker>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let pending = REGISTRATIONS.with(|entries| {
            entries.borrow().get(&label).and_then(|entry| {
                entry
                    .control
                    .pending
                    .lock()
                    .ok()
                    .and_then(|pending| pending.clone())
            })
        });
        let _ = tx.send(pending);
    })
    .map_err(|_| "File picker view is unavailable")?;
    match rx.await.map_err(|_| "File picker observation was lost")? {
        Some(pending) => Ok(pending.picker.lock().await.clone()),
        None => Ok(None),
    }
}

pub(crate) async fn cancel_and_wait(view: &tauri::Webview) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let control = REGISTRATIONS.with(|entries| {
            entries
                .borrow()
                .get(&label)
                .map(|entry| entry.control.clone())
        });
        if let Some(control) = &control {
            control.cancel();
        }
        let _ = tx.send(control);
    })
    .map_err(|_| "File request view is unavailable")?;
    if let Some(control) = rx.await.map_err(|_| "File request cancellation was lost")? {
        let pending = control
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(pending) = pending {
            pending.close().await?;
        }
    }
    Ok(())
}

pub(crate) async fn install(
    view: &tauri::Webview,
    automation: Arc<AsyncMutex<TabAutomation>>,
    locked: Arc<AtomicBool>,
    initial_directory: PathBuf,
) -> Result<(), String> {
    let label = view.label().to_owned();
    let owner = view.clone();
    let control = Arc::new(Control {
        visible: Arc::new(AtomicBool::new(false)),
        closed: Arc::new(AtomicBool::new(false)),
        locked,
        pending: Mutex::new(None),
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if installed(&label) {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_UNEXPECTED,
                ));
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            let mut entry = Registration {
                control: control.clone(),
                events: vec![],
            };
            for method in [
                "Page.fileChooserOpened",
                "Page.frameNavigated",
                "Page.frameDetached",
            ] {
                let control = control.clone();
                let view = owner.clone();
                let automation = automation.clone();
                let initial_directory = initial_directory.clone();
                let handler =
                    DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                        if method != "Page.fileChooserOpened" {
                            let frame = args.as_ref().and_then(|args| {
                                let mut raw = PWSTR::null();
                                let status = unsafe { args.ParameterObjectAsJson(&mut raw) };
                                let data = super::event_string(raw, 64 * 1024)?;
                                status.ok()?;
                                let value: serde_json::Value = serde_json::from_str(&data).ok()?;
                                (if method == "Page.frameNavigated" {
                                    &value["frame"]["id"]
                                } else {
                                    &value["frameId"]
                                })
                                .as_str()
                                .map(str::to_owned)
                            });
                            control.document_changed(frame.as_deref());
                            return Ok(());
                        }
                        if !control.permitted() {
                            return Ok(());
                        }
                        let Some(choice) = args.as_ref().and_then(|args| from_event(args).ok())
                        else {
                            control.cancel();
                            return Ok(());
                        };
                        start_request(
                            view.clone(),
                            automation.clone(),
                            control.clone(),
                            initial_directory.clone(),
                            choice,
                        );
                        Ok(())
                    }));
                let event = unsafe {
                    core.GetDevToolsProtocolEventReceiver(PCWSTR(HSTRING::from(method).as_ptr()))?
                };
                let mut token = 0;
                unsafe {
                    event.add_DevToolsProtocolEventReceived(&handler, &mut token)?;
                }
                entry.events.push((event, token));
            }
            REGISTRATIONS.with(|entries| entries.borrow_mut().insert(label, entry));
            Ok(())
        })()
        .map_err(|_| "Native user file chooser could not be installed".to_owned());
        let _ = tx.send(result);
    })
    .map_err(|_| "Browser view is unavailable")?;
    rx.await
        .map_err(|_| "File chooser installation was lost")??;
    super::protocol_call(view, "Page.enable", json!({})).await?;
    super::protocol_call(
        view,
        "Page.setInterceptFileChooserDialog",
        json!({"enabled":true}),
    )
    .await?;
    Ok(())
}

fn start_request(
    view: tauri::Webview,
    automation: Arc<AsyncMutex<TabAutomation>>,
    control: Arc<Control>,
    directory: PathBuf,
    choice: Choice,
) {
    let pending = {
        let mut current = control
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !control.permitted() {
            return;
        }
        if let Some(previous) = current.as_ref() {
            if !matches!(previous.done.borrow().as_ref(), Some(Ok(()))) {
                // Do not leave the earlier input eligible after a second chooser.
                previous.cancel.cancel();
                return;
            }
        }
        let (done, _) = watch::channel(None);
        let pending = Arc::new(Pending {
            cancel: CancellationToken::new(),
            done,
            picker: AsyncMutex::new(None),
            worker_failed: AtomicBool::new(false),
            frames: Mutex::new(None),
        });
        *current = Some(pending.clone());
        pending
    };
    let work = pending.clone();
    let worker = tauri::async_runtime::spawn(async move {
        run(view, automation, control, work, directory, choice).await
    });
    tauri::async_runtime::spawn(async move {
        match worker.await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::debug!("User file selection ended without delivery"),
            Err(_) => {
                pending.worker_failed.store(true, Ordering::Release);
            }
        }
        let result = pending.cleanup().await.and_then(|()| {
            if pending.worker_failed.load(Ordering::Acquire) {
                Err("File request worker failed".into())
            } else {
                Ok(())
            }
        });
        pending.done.send_replace(Some(result));
    });
}

struct FileTarget {
    route: OwnedFrameRoute,
    object: String,
    group: String,
}
impl FileTarget {
    async fn prepare(route: OwnedFrameRoute, choice: &Choice) -> Result<Self, String> {
        let group = format!("nomifun-user-file-{}", uuid::Uuid::new_v4());
        let world=route.command("Page.createIsolatedWorld",json!({"frameId":choice.frame,"worldName":"nomifun-user-file-picker","grantUniveralAccess":false})).await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or("File chooser document is unavailable")?;
        let resolved=route.command("DOM.resolveNode",json!({"backendNodeId":choice.backend_node,"executionContextId":context,"objectGroup":group})).await?;
        let object = resolved["object"]["objectId"]
            .as_str()
            .ok_or("File chooser input is unavailable")?
            .to_owned();
        let target = Self {
            route,
            object,
            group,
        };
        if let Err(error) = target.validate(0).await {
            target.release().await;
            return Err(error);
        }
        Ok(target)
    }
    async fn validate(&self, count: usize) -> Result<(), String> {
        let result=self.route.command("Runtime.callFunctionOn",json!({"objectId":self.object,
            "functionDeclaration":"function(count){return this instanceof HTMLInputElement && this.ownerDocument===document && this.type==='file' && !this.disabled && !this.webkitdirectory && (this.multiple||count<2);}",
            "arguments":[{"value":count}],"returnByValue":true})).await?;
        if result["result"]["value"] != true {
            return Err("File chooser input changed".into());
        }
        Ok(())
    }
    async fn release(&self) {
        let _ = self
            .route
            .command(
                "Runtime.releaseObjectGroup",
                json!({"objectGroup":self.group}),
            )
            .await;
    }
    async fn accepted_extensions(&self) -> Result<Vec<String>, String> {
        let result=self.route.command("Runtime.callFunctionOn",json!({"objectId":self.object,
            "functionDeclaration":"function(){return this.accept.slice(0,4097);}","returnByValue":true})).await?;
        let accept = result["result"]["value"]
            .as_str()
            .ok_or("File input type hint is unavailable")?;
        Ok(super::file_accept::extensions(accept))
    }
}
async fn run(
    view: tauri::Webview,
    automation: Arc<AsyncMutex<TabAutomation>>,
    control: Arc<Control>,
    pending: Arc<Pending>,
    directory: PathBuf,
    choice: Choice,
) -> Result<(), String> {
    let route = {
        let mut driver = tokio::select! { biased; _=pending.cancel.cancelled()=>return Ok(()), driver=automation.lock()=>driver };
        if !control.permitted() {
            return Ok(());
        }
        driver.user_file_route(&view, &choice).await?
    };
    if pending.cancel.is_cancelled() || !control.permitted() {
        return Ok(());
    }
    *pending
        .frames
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(route.document_lineage());
    let target = FileTarget::prepare(route, &choice).await?;
    let outcome = async {
        let extensions = target.accepted_extensions().await?;
        if pending.cancel.is_cancelled() || !control.permitted() {
            return Ok(());
        }
        let picker = NativeFilePicker::start(Options {
            title: "NomiFun — Select files".into(),
            initial_directory: directory,
            mode: PickerMode::Open { multiple: choice.multiple },
            extensions,
        })?;
        *pending.picker.lock().await = Some(picker.clone());
        let selected = tokio::select! {
            biased;
            _=pending.cancel.cancelled()=>{ picker.close().await?; None },
            result=picker.finished()=>result?,
        };
        picker.close().await?;
        let Some(paths) = selected else {
            return Ok(());
        };
        if pending.cancel.is_cancelled() || !control.permitted() {
            return Ok(());
        }
        choice
            .require_current()
            .map_err(|error| error.to_string())?;
        target.validate(paths.len()).await?;
        if pending.cancel.is_cancelled() || !control.permitted() {
            return Ok(());
        }
        target
            .route
            .set_user_files(
                &target.object,
                &paths,
                super::UserFileCommandGuard {
                    cancel: pending.cancel.clone(),
                    locked: control.locked.clone(),
                    visible: control.visible.clone(),
                    closed: control.closed.clone(),
                },
            )
            .await?;
        Ok(())
    }
    .await;
    target.release().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> (Control, Arc<Pending>) {
        let (done, _) = watch::channel(None);
        let pending = Arc::new(Pending {
            cancel: CancellationToken::new(),
            done,
            picker: AsyncMutex::new(None),
            worker_failed: AtomicBool::new(false),
            frames: Mutex::new(Some(vec!["child".into(), "root".into()])),
        });
        let control = Control {
            visible: Arc::new(AtomicBool::new(true)),
            closed: Arc::new(AtomicBool::new(false)),
            locked: Arc::new(AtomicBool::new(false)),
            pending: Mutex::new(Some(pending.clone())),
        };
        (control, pending)
    }
    #[test]
    fn only_request_document_or_ancestors_invalidate_selection() {
        let (control, pending) = request();
        control.document_changed(Some("advertisement"));
        assert!(!pending.cancel.is_cancelled());
        control.document_changed(Some("root"));
        assert!(pending.cancel.is_cancelled());
        let (control, pending) = request();
        control.document_changed(Some("child"));
        assert!(pending.cancel.is_cancelled());
        let (control, pending) = request();
        control.document_changed(None);
        assert!(pending.cancel.is_cancelled());
    }
    #[test]
    fn request_admission_requires_visible_idle_live_browser() {
        let (control, _) = request();
        assert!(control.permitted());
        control.locked.store(true, Ordering::Release);
        assert!(!control.permitted());
        control.locked.store(false, Ordering::Release);
        control.visible.store(false, Ordering::Release);
        assert!(!control.permitted());
        control.visible.store(true, Ordering::Release);
        control.closed.store(true, Ordering::Release);
        assert!(!control.permitted());
    }
}
