//! Profile clearing on an owned inert native view. Never a page/Agent API.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use nomifun_browser_platform::{runtime::WorkspaceError, run_guard::RunAdmissionError};
use tokio::sync::oneshot;
use tauri::Manager;
use webview2_com::{ClearBrowsingDataCompletedHandler, ProcessFailedEventHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2_13, ICoreWebView2Profile2,
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE, COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE,
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED}};
use windows::core::Interface;

type Sender = Rc<RefCell<Option<oneshot::Sender<Result<(), WorkspaceError>>>>>;
struct Pending { label: String, core: ICoreWebView2, _profile: ICoreWebView2Profile2, token: i64, sender: Sender }
impl Drop for Pending {
    fn drop(&mut self) { let _ = unsafe { self.core.remove_ProcessFailed(self.token) }; }
}
thread_local! { static PENDING: RefCell<HashMap<uuid::Uuid, Pending>> = RefCell::default(); }
fn finish(id: uuid::Uuid, result: Result<(), WorkspaceError>) {
    let entry = PENDING.with(|pending| pending.borrow_mut().remove(&id));
    if let Some(entry) = entry {
        let sender = entry.sender.borrow_mut().take();
        drop(entry);
        if let Some(sender) = sender { let _ = sender.send(result); }
    }
}

/// The caller retains this view and the Runtime's operation/creation ownership
/// until this future settles. Do not race it against a timeout or cancellation:
/// closing a controller is not proof that an in-flight Profile clear stopped.
pub(crate) async fn clear(view: &tauri::Webview, cancel: tokio_util::sync::CancellationToken) -> Result<(), WorkspaceError> {
    let id = uuid::Uuid::now_v7();
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        if cancel.is_cancelled() { let _ = tx.send(Err(RunAdmissionError::Cancelled.into())); return; }
        let sender: Sender = Rc::new(RefCell::new(Some(tx)));
        let result = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;
            let profile = core.cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile2>()?;
            let failed = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    args.ProcessFailedKind(&mut kind)?;
                    // Renderer/worker failure alone cannot prove a browser-side
                    // clearing operation ended. Only the browser's exit does.
                    if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                        finish(id, Err(WorkspaceError::NativeCommandFailed));
                    }
                }
                Ok(())
            }));
            let mut token = 0;
            core.add_ProcessFailed(&failed, &mut token)?;
            PENDING.with(|pending| pending.borrow_mut().insert(id, Pending { label, core, _profile:profile.clone(), token, sender:sender.clone() }));
            let completed = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
                finish(id, result.map_err(|_|WorkspaceError::NativeCommandFailed));
                Ok(())
            }));
            profile.ClearBrowsingData(COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE, &completed)
        })();
        if result.is_err() {
            // Dispatch failed, so there is no unacknowledged native operation.
            finish(id, Err(WorkspaceError::NativeCommandFailed));
            if let Some(sender) = sender.borrow_mut().take() { let _ = sender.send(Err(WorkspaceError::NativeCommandFailed)); }
        }
    }).map_err(|_|WorkspaceError::NativeCommandFailed)?;
    // Losing the callback channel is not completion: the runtime retains its
    // maintenance view and fences reuse until explicit native teardown.
    rx.await.map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
}

/// Called under the Runtime creation guard before destroying its maintenance
/// controller, including recovery from a panicked/dropped work task. A native
/// callback remains owned in PENDING even if its original Rust waiter is gone.
pub(crate) async fn settle(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    loop {
        let label=view.label().to_owned();
        let (tx,rx)=oneshot::channel();
        // This UI dispatch is also a barrier behind a previously queued clear.
        view.app_handle().run_on_main_thread(move || {
            let pending=PENDING.with(|entries|entries.borrow().values().any(|entry|entry.label==label));
            let _=tx.send(pending);
        }).map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?;
        let pending=rx.await.map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?;
        if !pending { return Ok(()); }
        // Neither elapsed time nor controller Close is completion evidence.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
