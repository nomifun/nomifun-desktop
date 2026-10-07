//! One operation's native HTML file-chooser event. It contains no file paths
//! or upload authority; the owning semantic world must validate its target.
use nomifun_browser_platform::{run_guard::RunAdmissionError, runtime::WorkspaceError};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use webview2_com::{DevToolsProtocolEventReceivedEventHandler, Microsoft::Web::WebView2::Win32::*};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

pub(crate) struct Choice {
    invalid: Arc<AtomicBool>,
    pub session: String,
    pub frame: String,
    pub backend_node: i64,
    pub multiple: bool,
}
impl Choice {
    pub(crate) fn require_current(&self) -> Result<(), WorkspaceError> {
        if self.invalid.load(Ordering::Acquire) {
            Err(WorkspaceError::ActionInterrupted)
        } else {
            Ok(())
        }
    }
}
fn choice(session: String, data: Value) -> Result<Choice, WorkspaceError> {
    let frame = data["frameId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or(WorkspaceError::UnsupportedAction)?
        .to_owned();
    let backend_node = data["backendNodeId"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or(WorkspaceError::UnsupportedAction)?;
    let multiple = match data["mode"].as_str() {
        Some("selectSingle") => false,
        Some("selectMultiple") => true,
        _ => return Err(WorkspaceError::UnsupportedAction),
    };
    Ok(Choice {
        invalid: Arc::new(AtomicBool::new(false)),
        session,
        frame,
        backend_node,
        multiple,
    })
}
pub(super) fn from_event(args: &ICoreWebView2DevToolsProtocolEventReceivedEventArgs) -> Result<Choice, WorkspaceError> {
    let mut raw = PWSTR::null();
    let args2 = args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let status = unsafe { args2.SessionId(&mut raw) };
    let session = super::event_string(raw, 256).ok_or(WorkspaceError::NativeCommandFailed)?;
    status.map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let mut raw = PWSTR::null();
    let status = unsafe { args.ParameterObjectAsJson(&mut raw) };
    let parameters = super::event_string(raw, 8192).ok_or(WorkspaceError::NativeCommandFailed)?;
    status.map_err(|_| WorkspaceError::NativeCommandFailed)?;
    choice(session, serde_json::from_str(&parameters).map_err(|_| WorkspaceError::NativeCommandFailed)?)
}
struct Registration {
    invalid: Arc<AtomicBool>,
    label: String,
    receiver: ICoreWebView2DevToolsProtocolEventReceiver,
    token: i64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.invalid.store(true, Ordering::Release);
        let _ = unsafe {
            self.receiver
                .remove_DevToolsProtocolEventReceived(self.token)
        };
    }
}
thread_local! {static REGISTRATIONS:RefCell<HashMap<uuid::Uuid,Registration>>=RefCell::default();}
pub(super) fn close_view(label: &str) {
    REGISTRATIONS.with(|entries| {
        entries
            .borrow_mut()
            .retain(|_, registration| registration.label != label)
    });
}
pub(crate) struct FileChooser {
    view: tauri::Webview,
    id: uuid::Uuid,
    armed: Arc<AtomicBool>,
    invalid: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Choice, WorkspaceError>>,
}
impl Drop for FileChooser {
    fn drop(&mut self) {
        self.armed.store(false, Ordering::Release);
        let id = self.id;
        let _ = self.view.with_webview(move |_| {
            REGISTRATIONS.with(|entries| entries.borrow_mut().remove(&id));
        });
    }
}
impl FileChooser {
    pub(crate) async fn listen(view: &tauri::Webview) -> Result<Self, WorkspaceError> {
        let id = uuid::Uuid::new_v4();
        let label = view.label().to_owned();
        let armed = Arc::new(AtomicBool::new(false));
        let invalid = Arc::new(AtomicBool::new(false));
        let arm = armed.clone();
        let fail = invalid.clone();
        let registered_invalid = invalid.clone();
        let seen = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel(1);
        let chooser = Self {
            view: view.clone(),
            id,
            armed,
            invalid,
            receiver,
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let installed = (|| -> windows::core::Result<()> {
                let core = unsafe { platform.controller().CoreWebView2()? };
                let receiver = unsafe {
                    core.GetDevToolsProtocolEventReceiver(PCWSTR(
                        HSTRING::from("Page.fileChooserOpened").as_ptr(),
                    ))?
                };
                let handler =
                    DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                        if !arm.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        if seen.swap(true, Ordering::AcqRel) {
                            fail.store(true, Ordering::Release);
                            return Ok(());
                        }
                        let received = (|| -> Result<Choice, WorkspaceError> {
                            let args = args.ok_or(WorkspaceError::NativeCommandFailed)?;
                            let mut choice = from_event(&args)?;
                            choice.invalid = fail.clone();
                            Ok(choice)
                        })();
                        if sender.try_send(received).is_err() {
                            fail.store(true, Ordering::Release);
                        }
                        Ok(())
                    }));
                let mut token = 0;
                unsafe {
                    receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)?;
                }
                REGISTRATIONS.with(|entries| {
                    entries.borrow_mut().insert(
                        id,
                        Registration {
                            invalid: registered_invalid,
                            label,
                            receiver,
                            token,
                        },
                    )
                });
                Ok(())
            })()
            .map_err(|_| WorkspaceError::NativeCommandFailed);
            let _ = tx.send(installed);
        })
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        rx.await
            .map_err(|_| WorkspaceError::NativeCommandFailed)??;
        Ok(chooser)
    }
    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }
    pub(crate) async fn next(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Choice, WorkspaceError> {
        let result=tokio::select! {
            biased;
            _=cancel.cancelled()=>return Err(RunAdmissionError::Cancelled.into()),
            result=tokio::time::timeout(std::time::Duration::from_secs(3),self.receiver.recv())=>result,
        }.map_err(|_|WorkspaceError::ActionInterrupted)?.ok_or(WorkspaceError::ActionInterrupted)??;
        if self.invalid.load(Ordering::Acquire) {
            return Err(WorkspaceError::ActionInterrupted);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chooser_requires_an_exact_html_input_target() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"frameId":"frame","mode":"selectSingle"}),
            serde_json::json!({"frameId":"frame","mode":"directory","backendNodeId":1}),
        ] {
            assert!(choice(String::new(), value).is_err());
        }
        let selected = choice(
            "owned-session".into(),
            serde_json::json!({"frameId":"frame","mode":"selectMultiple","backendNodeId":42}),
        )
        .unwrap();
        assert!(selected.multiple);
        assert_eq!(selected.backend_node, 42);
        assert_eq!(selected.session, "owned-session");
    }
}
