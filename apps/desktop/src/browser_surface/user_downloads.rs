//! UserReady downloads from the same native WebView. COM state stays on its UI
//! thread; an isolated Save dialog supplies the only destination authority.
use super::user_file_picker::{NativeFilePicker, Options, PickerMode, valid_save_name};
use nomifun_browser_platform::{
    revision::BrowserRevision,
    runtime::{
        BrowserDownloadSnapshot, BrowserDownloadState, BrowserTabSnapshot, BrowserTabTarget,
    },
};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use webview2_com::{
    BytesReceivedChangedEventHandler, DownloadStartingEventHandler,
    Microsoft::Web::WebView2::Win32::*, StateChangedEventHandler,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};
#[path = "agent_downloads.rs"]
mod agent_downloads;
pub(crate) use agent_downloads::{
    AgentRequest, arm_agent, capture_agent, finish_agent, inherit_agent, settle_agent, wait_page_downloads,
};
use nomifun_browser_platform::{downloads::PreparedBrowserDownload, runtime::WorkspaceError};

const MAX_DOWNLOADS: usize = 4;
const MAX_HISTORY: usize = 64;
pub(crate) struct DownloadHistory {
    entries: Mutex<Vec<BrowserDownloadSnapshot>>,
    revision: Arc<BrowserRevision>,
}
impl DownloadHistory {
    pub(crate) fn new(revision: Arc<BrowserRevision>) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(vec![]),
            revision,
        })
    }
    pub(crate) fn snapshot(&self) -> Vec<BrowserDownloadSnapshot> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn insert(&self, job: &Job, filename: String) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= MAX_HISTORY {
            if let Some(index) = entries.iter().position(|entry| {
                !entry.can_cancel
                    && matches!(
                        entry.state,
                        BrowserDownloadState::Completed
                            | BrowserDownloadState::Cancelled
                            | BrowserDownloadState::Failed
                    )
            }) {
                entries.remove(index);
            } else {
                return;
            } // Active jobs are already bounded by runtime/tab limits.
        }
        entries.push(BrowserDownloadSnapshot {
            id: job.id.clone(),
            tab_id: job.target.tab_id.clone(),
            filename,
            state: BrowserDownloadState::Choosing,
            received_bytes: 0,
            total_bytes: None,
            can_cancel: true,
        });
        self.revision.bump();
    }
    fn update(&self, id: &str, update: impl FnOnce(&mut BrowserDownloadSnapshot)) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
            let before = entry.clone();
            update(entry);
            if *entry != before {
                self.revision.bump();
            }
        }
    }
}
struct Job {
    download: Option<std::sync::Weak<PreparedBrowserDownload>>,
    filename: String,
    policy_error: Mutex<Option<WorkspaceError>>,
    id: String,
    target: BrowserTabTarget,
    cancel: CancellationToken,
    started: AtomicBool,
    cancelling_native: AtomicBool,
    cancel_acknowledged: AtomicBool,
    picker: Mutex<Option<NativeFilePicker>>,
    native: watch::Sender<Option<bool>>,
    done: watch::Sender<Option<Result<(), String>>>,
    history: Arc<DownloadHistory>,
    failed: AtomicBool,
}
impl Job {
    #[cfg(test)]
    fn new(target: BrowserTabTarget, history: Arc<DownloadHistory>) -> Arc<Self> {
        Self::with_download(
            target,
            history,
            None,
            CancellationToken::new(),
            String::new(),
        )
    }
    fn with_download(
        target: BrowserTabTarget,
        history: Arc<DownloadHistory>,
        download: Option<Arc<PreparedBrowserDownload>>,
        cancel: CancellationToken,
        filename: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            download: download.as_ref().map(Arc::downgrade),
            filename,
            policy_error: Mutex::new(None),
            id: uuid::Uuid::now_v7().to_string(),
            target,
            cancel,
            started: AtomicBool::new(false),
            cancelling_native: AtomicBool::new(false),
            cancel_acknowledged: AtomicBool::new(false),
            picker: Mutex::new(None),
            native: watch::channel(None).0,
            done: watch::channel(None).0,
            history,
            failed: AtomicBool::new(false),
        })
    }
    fn cancel(&self) {
        self.cancel.cancel();
        if self.done.borrow().is_none() {
            self.history.update(&self.id, |entry| {
                // Cleanup may publish a terminal record between the done check
                // and this lock; cancellation must not regress that record.
                if matches!(
                    entry.state,
                    BrowserDownloadState::Choosing | BrowserDownloadState::InProgress
                ) {
                    entry.state = BrowserDownloadState::Cancelling;
                    entry.can_cancel = false;
                }
            });
        }
    }
    fn finish(&self, result: Result<(), String>) {
        if self.download.is_some() && result.is_ok() {
            // Native completion is not artifact publication proof.
            self.done.send_replace(Some(result));
            return;
        }
        self.history.update(&self.id, |entry| {
            entry.state = if result.is_err() {
                BrowserDownloadState::Failed
            } else if *self.native.borrow() == Some(true) {
                BrowserDownloadState::Completed
            } else if self.failed.load(Ordering::Acquire) {
                BrowserDownloadState::Failed
            } else {
                BrowserDownloadState::Cancelled
            };
            entry.can_cancel = result.is_err();
        });
        self.done.send_replace(Some(result));
    }
    async fn wait(&self) -> Result<(), String> {
        let mut done = self.done.subscribe();
        loop {
            if let Some(result) = done.borrow().clone() {
                return result;
            }
            done.changed()
                .await
                .map_err(|_| "Download cleanup result was lost")?;
        }
    }
    async fn close_picker(&self) -> Result<(), String> {
        let picker = self
            .picker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(picker) = picker {
            picker.close().await?;
        }
        self.picker.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok(())
    }
}
struct Control {
    agent: Mutex<Option<agent_downloads::Admission>>,
    visible: AtomicBool,
    closed: AtomicBool,
    locked: Arc<AtomicBool>,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    jobs: Mutex<Vec<Arc<Job>>>,
}
impl Control {
    fn permitted(&self) -> bool {
        self.visible.load(Ordering::Acquire)
            && !self.closed.load(Ordering::Acquire)
            && !self.locked.load(Ordering::Acquire)
    }
    fn cancel(&self, all: bool) -> Vec<Arc<Job>> {
        let jobs: Vec<_> = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|job| all || (job.download.is_none() && !job.started.load(Ordering::Acquire)))
            .cloned()
            .collect();
        for job in &jobs {
            job.cancel();
        }
        jobs
    }
}
struct NativeDownload {
    args: ICoreWebView2DownloadStartingEventArgs,
    operation: ICoreWebView2DownloadOperation,
    deferral: RefCell<Option<ICoreWebView2Deferral>>,
    state_token: Option<i64>,
    bytes_token: Option<i64>,
}
impl Drop for NativeDownload {
    fn drop(&mut self) {
        unsafe {
            if let Some(token) = self.state_token {
                let _ = self.operation.remove_StateChanged(token);
            }
            if let Some(token) = self.bytes_token {
                let _ = self.operation.remove_BytesReceivedChanged(token);
            }
            // Unexpected owner loss still requests cancellation. Only the
            // explicit async cleanup path provides terminal proof.
            let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
            if self.operation.State(&mut state).is_err()
                || state != COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED
            {
                let _ = self.operation.Cancel();
            }
            if let Some(deferral) = self.deferral.borrow_mut().take() {
                let _ = self.args.SetCancel(true);
                let _ = deferral.Complete();
            }
        }
    }
}
struct Registration {
    core: ICoreWebView2_4,
    token: i64,
    control: Arc<Control>,
    downloads: HashMap<String, Rc<NativeDownload>>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.control.closed.store(true, Ordering::Release);
        self.control.cancel(true);
        let _ = unsafe { self.core.remove_DownloadStarting(self.token) };
    }
}
thread_local! { static REGISTRATIONS: RefCell<HashMap<String, Registration>> = RefCell::default(); }
fn control(label: &str) -> Option<Arc<Control>> {
    REGISTRATIONS.with(|entries| {
        entries
            .borrow()
            .get(label)
            .map(|entry| entry.control.clone())
    })
}
pub(crate) fn set_visible(label: &str, visible: bool) {
    if let Some(control) = control(label) {
        control.visible.store(visible, Ordering::Release);
        if !visible {
            control.cancel(false);
        }
    }
}
pub(crate) fn cancel_pending(label: &str) {
    if let Some(control) = control(label) {
        control.cancel(false);
    }
}
pub(super) fn close_view(label: &str) {
    let entry = REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label));
    drop(entry);
}
fn publish(operation: &ICoreWebView2DownloadOperation, job: &Job) -> windows::core::Result<()> {
    let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
    let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
    let mut resumable = windows::core::BOOL::default();
    unsafe {
        operation.State(&mut state)?;
        operation.InterruptReason(&mut reason)?;
        operation.CanResume(&mut resumable)?;
    }
    let terminal = terminal_state(
        state,
        reason,
        resumable.as_bool(),
        job.cancelling_native.load(Ordering::Acquire),
        job.cancel_acknowledged.load(Ordering::Acquire),
    );
    // A later progress notification must not erase a witnessed terminal result.
    // Only starting an explicit Cancel clears prior interruption evidence.
    if terminal.is_some() {
        job.native.send_replace(terminal);
    }
    let (mut received, mut total) = (0, 0);
    unsafe {
        operation.BytesReceived(&mut received)?;
        operation.TotalBytesToReceive(&mut total)?;
    }
    if let Some(download) = job.download.as_ref().and_then(std::sync::Weak::upgrade) {
        if let Err(error) =
            download.progress(received.max(0) as u64, (total > 0).then_some(total as u64))
        {
            *job.policy_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
            job.cancel();
        }
    }
    job.history.update(&job.id, |entry| {
        entry.received_bytes = received.max(0) as u64;
        entry.total_bytes = (total > 0).then_some(total as u64);
        if job.started.load(Ordering::Acquire)
            && !job.cancel.is_cancelled()
            && job.done.borrow().is_none()
            && matches!(
                entry.state,
                BrowserDownloadState::Choosing | BrowserDownloadState::InProgress
            )
        {
            entry.state = BrowserDownloadState::InProgress;
        }
    });
    Ok(())
}
fn terminal_state(
    state: COREWEBVIEW2_DOWNLOAD_STATE,
    reason: COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON,
    resumable: bool,
    cancelling: bool,
    acknowledged: bool,
) -> Option<bool> {
    if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
        return Some(true);
    }
    // WebView2 152 reproduces State=IN_PROGRESS after a truncated response has
    // reached SERVER_CONTENT_LENGTH_MISMATCH and CanResume=false, even after
    // Cancel succeeds. Normalize only this observed fatal combination; never
    // infer completion from an arbitrary error, an idle timer, or file size.
    let length_mismatch = state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
        && reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CONTENT_LENGTH_MISMATCH
        && !resumable;
    if state != COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED && !length_mismatch {
        return None;
    }
    if !cancelling {
        return Some(false);
    } // Requests cancellation, not cleanup proof.
    if !acknowledged {
        return None;
    }
    let fatal = matches!(
        reason,
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CONTENT_LENGTH_MISMATCH
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_DISCONNECTED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_SERVER_DOWN
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_INVALID_REQUEST
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FAILED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_BAD_CONTENT
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_UNAUTHORIZED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FORBIDDEN
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CERTIFICATE_PROBLEM
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_UNEXPECTED_RESPONSE
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CROSS_ORIGIN_REDIRECT
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_FAILED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_ACCESS_DENIED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NAME_TOO_LONG
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_LARGE
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_MALICIOUS
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_BLOCKED_BY_POLICY
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_SECURITY_CHECK_FAILED
    );
    if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED || (!resumable && fatal) {
        Some(false)
    } else {
        None
    }
}
async fn change(
    view: &tauri::Webview,
    id: &str,
    destination: Option<PathBuf>,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (label, id) = (view.label().to_owned(), id.to_owned());
    view.with_webview(move |_| {
        let found = REGISTRATIONS.with(|entries| {
            entries.borrow().get(&label).and_then(|entry| {
                let job = entry
                    .control
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .find(|job| job.id == id)
                    .cloned()?;
                Some((
                    entry.downloads.get(&id)?.clone(),
                    entry.control.clone(),
                    job,
                ))
            })
        });
        let result = (|| -> Result<(), String> {
            let (native, control, job) = found.ok_or("Download owner is unavailable")?;
            let activating = destination.is_some();
            unsafe {
                if let Some(path) = destination {
                    let admitted = if job.download.is_some() {
                        control.locked.load(Ordering::Acquire)
                            && !control.closed.load(Ordering::Acquire)
                    } else {
                        control.permitted()
                    };
                    if !admitted
                        || job.cancel.is_cancelled()
                        || control
                            .metadata
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .target
                            != job.target
                    {
                        return Err("Download save request is no longer current".into());
                    }
                    if !path.is_absolute() {
                        return Err("Download destination is not absolute".into());
                    }
                    if let Some(name) = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|_| job.download.is_none())
                    {
                        job.history
                            .update(&job.id, |entry| entry.filename = name.to_owned());
                    }
                    let path = path.to_str().ok_or("Download path is not Unicode")?;
                    native
                        .args
                        .SetResultFilePath(PCWSTR(HSTRING::from(path).as_ptr()))
                        .map_err(|_| "Download destination failed")?;
                    native
                        .args
                        .SetCancel(false)
                        .map_err(|_| "Download admission failed")?;
                } else {
                    if native.deferral.borrow().is_some() {
                        native
                            .args
                            .SetCancel(true)
                            .map_err(|_| "Download cancellation failed")?;
                    }
                    let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                    native
                        .operation
                        .State(&mut state)
                        .map_err(|_| "Download state is unavailable")?;
                    if state != COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                        job.cancelling_native.store(true, Ordering::Release);
                        job.cancel_acknowledged.store(false, Ordering::Release);
                        job.native.send_replace(None);
                        native
                            .operation
                            .Cancel()
                            .map_err(|_| "Native download did not cancel")?;
                        job.cancel_acknowledged.store(true, Ordering::Release);
                    }
                }
                let deferral = native.deferral.borrow_mut().take();
                if let Some(deferral) = deferral {
                    deferral
                        .Complete()
                        .map_err(|_| "Download deferral did not settle")?;
                }
            }
            publish(&native.operation, &job)
                .map_err(|_| "Download state is unavailable".to_owned())?;
            if activating {
                job.started.store(true, Ordering::Release);
                job.history.update(&job.id, |entry| {
                    if !job.cancel.is_cancelled() {
                        entry.state = BrowserDownloadState::InProgress;
                    }
                });
            }
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| "Download view is unavailable")?;
    rx.await.map_err(|_| "Download native callback was lost")?
}
async fn wait_native(job: &Job) -> Result<bool, String> {
    let mut state = job.native.subscribe();
    loop {
        if let Some(done) = *state.borrow() {
            return Ok(done);
        }
        state
            .changed()
            .await
            .map_err(|_| "Download state channel was lost")?;
    }
}
async fn cleanup(view: &tauri::Webview, job: &Job) -> Result<(), String> {
    job.close_picker().await?;
    if *job.native.borrow() != Some(true) {
        change(view, &job.id, None).await?;
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), wait_native(job))
        .await
        .map_err(|_| "Download cancellation has no terminal confirmation")??;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (label, id) = (view.label().to_owned(), job.id.clone());
    view.with_webview(move |_| {
        let native = REGISTRATIONS.with(|entries| {
            entries
                .borrow_mut()
                .get_mut(&label)
                .and_then(|entry| entry.downloads.remove(&id))
        });
        drop(native);
        let _ = tx.send(());
    })
    .map_err(|_| "Download cleanup view is unavailable")?;
    rx.await
        .map_err(|_| "Download cleanup callback was lost".into())
}
async fn run(
    view: &tauri::Webview,
    control: &Control,
    job: &Job,
    directory: PathBuf,
    filename: String,
) -> Result<(), String> {
    if let Some(download) = job.download.as_ref().and_then(std::sync::Weak::upgrade) {
        if job.cancel.is_cancelled() {
            return Ok(());
        }
        change(view, &job.id, Some(download.native_path())).await?;
        tokio::select! { biased;
            _ = job.cancel.cancelled() => {},
            result = wait_native(job) => { if !result? { return Err("Native Agent download was interrupted".into()); } },
        }
        return Ok(());
    }
    if job.cancel.is_cancelled() || !control.permitted() {
        return Ok(());
    }
    let picker = NativeFilePicker::start(Options {
        title: "NomiFun — Save download".into(),
        initial_directory: directory,
        mode: PickerMode::Save { filename },
        extensions: vec![],
    })?;
    *job.picker.lock().unwrap_or_else(|e| e.into_inner()) = Some(picker.clone());
    let paths = tokio::select! { biased;
        _=job.cancel.cancelled()=>None,
        result=picker.finished()=>result?,
    };
    job.close_picker().await?;
    if let Some(mut paths) = paths {
        if paths.len() != 1 {
            return Err("Save picker returned an invalid destination count".into());
        }
        change(view, &job.id, paths.pop()).await?;
        tokio::select! { biased;
            _=job.cancel.cancelled()=>{},
            result=wait_native(job)=>{ if !result? { return Err("Native download was interrupted".into()); } },
        }
    }
    Ok(())
}
pub(crate) async fn cancel_and_wait(view: &tauri::Webview, closing: bool) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let jobs = control(&label)
            .map(|control| {
                if closing {
                    control.closed.store(true, Ordering::Release);
                }
                control.cancel(closing)
            })
            .unwrap_or_default();
        let _ = tx.send(jobs);
    })
    .map_err(|_| "Download cancellation view is unavailable")?;
    for job in rx
        .await
        .map_err(|_| "Download cancellation callback was lost")?
    {
        if job.wait().await.is_err() {
            cleanup(view, &job).await?;
            job.finish(Ok(()));
        }
    }
    Ok(())
}
pub(crate) async fn cancel_one(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    id: String,
) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 {
        return Err("Invalid download id".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let result = (|| {
            let control = control(&label).ok_or("Download owner is unavailable")?;
            // A renderer menu occludes the native surface; visibility is not authority.
            if control.closed.load(Ordering::Acquire)
                || control.locked.load(Ordering::Acquire)
                || control
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    != target
            {
                return Err("Download cancellation is no longer current");
            }
            let job = control
                .jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|job| job.id == id)
                .cloned()
                .ok_or("Download is no longer active")?;
            job.cancel();
            Ok(job)
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| "Download cancellation view is unavailable")?;
    let job = rx
        .await
        .map_err(|_| "Download cancellation callback was lost")??;
    if job.wait().await.is_err() {
        let result = cleanup(view, &job).await;
        job.finish(result.clone());
        result?;
    }
    Ok(())
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    locked: Arc<AtomicBool>,
    directory: PathBuf,
    history: Arc<DownloadHistory>,
) -> Result<(), String> {
    let control = Arc::new(Control {
        agent: Mutex::new(None),
        visible: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        locked,
        metadata,
        jobs: Mutex::new(vec![]),
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (label, owner) = (view.label().to_owned(), view.clone());
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if REGISTRATIONS.with(|entries| entries.borrow().contains_key(&label)) {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_UNEXPECTED,
                ));
            }
            let core: ICoreWebView2_4 = unsafe { platform.controller().CoreWebView2()? }.cast()?;
            let event_label = label.clone();
            let event_control = control.clone();
            let handler = DownloadStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                unsafe {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                }
                let agent = if event_control.locked.load(Ordering::Acquire) {
                    event_control
                        .agent
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .filter(|grant| {
                            grant.claim(
                                &event_control
                                    .metadata
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .target,
                            )
                        })
                        .map(|grant| grant.request)
                } else {
                    None
                };
                if !event_control.permitted() && agent.is_none() {
                    return Ok(());
                }
                {
                    let mut jobs = event_control.jobs.lock().unwrap_or_else(|e| e.into_inner());
                    jobs.retain(|job| !matches!(job.done.borrow().as_ref(), Some(Ok(()))));
                    if jobs.len() >= MAX_DOWNLOADS
                        || jobs.iter().any(|job| !job.started.load(Ordering::Acquire))
                    {
                        return Ok(());
                    }
                }
                let mut raw = PWSTR::null();
                unsafe {
                    args.ResultFilePath(&mut raw)?;
                }
                let suggested = super::event_string(raw, 32768)
                    .and_then(|path| {
                        PathBuf::from(path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .filter(|name| valid_save_name(name))
                    .unwrap_or_else(|| "download".into());
                let job = Job::with_download(
                    event_control
                        .metadata
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .target
                        .clone(),
                    history.clone(),
                    agent.as_ref().map(|request| request.file.clone()),
                    agent
                        .as_ref()
                        .map(|request| request.cancel.child_token())
                        .unwrap_or_default(),
                    suggested.clone(),
                );
                let operation = unsafe { args.DownloadOperation()? };
                let mut native = NativeDownload {
                    args: args.clone(),
                    operation,
                    deferral: RefCell::new(Some(unsafe { args.GetDeferral()? })),
                    state_token: None,
                    bytes_token: None,
                };
                let observed = job.clone();
                let state = StateChangedEventHandler::create(Box::new(move |operation, _| {
                    if let Some(operation) = operation {
                        publish(&operation, &observed)?;
                    }
                    Ok(())
                }));
                let mut state_token = 0;
                unsafe {
                    native
                        .operation
                        .add_StateChanged(&state, &mut state_token)?;
                }
                native.state_token = Some(state_token);
                let observed = job.clone();
                let bytes =
                    BytesReceivedChangedEventHandler::create(Box::new(move |operation, _| {
                        if let Some(operation) = operation {
                            publish(&operation, &observed)?;
                        }
                        Ok(())
                    }));
                let mut bytes_token = 0;
                unsafe {
                    native
                        .operation
                        .add_BytesReceivedChanged(&bytes, &mut bytes_token)?;
                }
                native.bytes_token = Some(bytes_token);
                let native = Rc::new(native);
                // Recheck after COM calls before publishing the request.
                let mut jobs = event_control.jobs.lock().unwrap_or_else(|e| e.into_inner());
                if (!event_control.permitted() && agent.is_none())
                    || event_control.closed.load(Ordering::Acquire)
                    || jobs.len() >= MAX_DOWNLOADS
                    || jobs.iter().any(|job| !job.started.load(Ordering::Acquire))
                {
                    drop(jobs);
                    drop(native);
                    return Ok(());
                }
                let inserted = REGISTRATIONS.with(|entries| {
                    if let Some(entry) = entries.borrow_mut().get_mut(&event_label) {
                        entry.downloads.insert(job.id.clone(), native.clone());
                        true
                    } else {
                        false
                    }
                });
                if !inserted {
                    drop(jobs);
                    drop(native);
                    return Ok(());
                }
                jobs.push(job.clone());
                drop(jobs);
                history.insert(&job, suggested.clone());
                if let Some(agent) = agent {
                    if nomi_browser_engine::download::is_executable_denylist(&suggested) {
                        *job.policy_error.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(WorkspaceError::DownloadDenied);
                        job.cancel();
                    }
                    agent.job.send_replace(Some(job.clone()));
                }
                let (view, control, directory) =
                    (owner.clone(), event_control.clone(), directory.clone());
                let worker_job = job.clone();
                let worker_view = view.clone();
                let worker = tauri::async_runtime::spawn(async move {
                    run(&worker_view, &control, &worker_job, directory, suggested).await
                });
                tauri::async_runtime::spawn(async move {
                    if !matches!(worker.await, Ok(Ok(()))) {
                        job.failed
                            .store(!job.cancel.is_cancelled(), Ordering::Release);
                        tracing::warn!("User browser download did not complete normally");
                    }
                    let result = cleanup(&view, &job).await;
                    job.finish(result);
                });
                Ok(())
            }));
            let mut token = 0;
            unsafe {
                core.add_DownloadStarting(&handler, &mut token)?;
            }
            REGISTRATIONS.with(|entries| {
                entries.borrow_mut().insert(
                    label,
                    Registration {
                        core,
                        token,
                        control,
                        downloads: HashMap::new(),
                    },
                )
            });
            Ok(())
        })()
        .map_err(|_| "Native download events are unavailable".to_owned());
        let _ = tx.send(result);
    })
    .map_err(|_| "Download view is unavailable")?;
    rx.await
        .map_err(|_| "Download installation callback was lost")?
}

/// Read-only conformance probes; never exposed to renderer IPC or Agent tools.
pub(crate) async fn inspect_native(
    view: &tauri::Webview,
) -> Result<Vec<serde_json::Value>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let handles = REGISTRATIONS.with(|entries| {
            entries.borrow().get(&label).map(|entry| entry.downloads.iter().map(|(id, native)| (id.clone(), native.clone(), entry.control.clone())).collect::<Vec<_>>()).unwrap_or_default()
        });
        let rows = handles.into_iter().map(|(id, native, control)| {
                let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
                let mut resumable = windows::core::BOOL::default();
                let mut received = 0;
                unsafe {
                    native.operation.State(&mut state)?;
                    native.operation.InterruptReason(&mut reason)?;
                    native.operation.CanResume(&mut resumable)?;
                    native.operation.BytesReceived(&mut received)?;
                }
                let job = control.jobs.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|job| job.id==id).cloned();
                Ok(serde_json::json!({"state":state.0,"reason":reason.0,"can_resume":resumable.as_bool(),"received":received,"job":job.map(|job|serde_json::json!({"started":job.started.load(Ordering::Acquire),"cancelled":job.cancel.is_cancelled(),"cancelling_native":job.cancelling_native.load(Ordering::Acquire),"native_terminal":*job.native.borrow(),"cleanup":*job.done.borrow()}))}))
            }).collect::<windows::core::Result<Vec<_>>>().map_err(|e| e.to_string());
        let _ = tx.send(rows);
    }).map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

/// Read-only conformance probes; never exposed to renderer IPC or Agent tools.
pub(crate) async fn inspect(
    view: &tauri::Webview,
) -> Result<(Option<NativeFilePicker>, usize, usize, usize), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let result = control(&label)
            .map(|control| {
                let jobs = control.jobs.lock().unwrap_or_else(|e| e.into_inner());
                let picker = jobs
                    .iter()
                    .find_map(|job| job.picker.lock().unwrap_or_else(|e| e.into_inner()).clone());
                let pending = jobs
                    .iter()
                    .filter(|job| job.done.borrow().is_none())
                    .count();
                let completed = jobs
                    .iter()
                    .filter(|job| {
                        *job.native.borrow() == Some(true)
                            && matches!(job.done.borrow().as_ref(), Some(Ok(())))
                    })
                    .count();
                let active = jobs
                    .iter()
                    .filter(|job| {
                        job.started.load(Ordering::Acquire)
                            && job.native.borrow().is_none()
                            && job.done.borrow().is_none()
                    })
                    .count();
                (picker, pending, completed, active)
            })
            .unwrap_or((None, 0, 0, 0));
        let _ = tx.send(result);
    })
    .map_err(|_| "Download inspection view is unavailable")?;
    rx.await
        .map_err(|_| "Download inspection callback was lost".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fatal_length_mismatch_requires_cancel_ack_but_never_claims_completed() {
        let state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
        let reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CONTENT_LENGTH_MISMATCH;
        assert_eq!(
            terminal_state(state, reason, false, false, false),
            Some(false)
        );
        assert_eq!(terminal_state(state, reason, false, true, false), None);
        assert_eq!(
            terminal_state(state, reason, false, true, true),
            Some(false)
        );
        assert_eq!(terminal_state(state, reason, true, true, true), None);
        assert_eq!(
            terminal_state(
                state,
                COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED,
                false,
                true,
                true
            ),
            None
        );
        for reason in [
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_NO_RANGE,
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_HASH_MISMATCH,
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_SHORT,
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE,
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON(999),
        ] {
            assert_eq!(
                terminal_state(
                    COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED,
                    reason,
                    false,
                    true,
                    true
                ),
                None
            );
        }
        assert_eq!(
            terminal_state(
                COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED,
                COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED,
                false,
                true,
                true
            ),
            Some(false)
        );
    }
    #[test]
    fn history_is_bounded_preserves_active_and_publishes_only_changes() {
        let revision = Arc::new(BrowserRevision::default());
        let history = DownloadHistory::new(revision.clone());
        let target = control().metadata.lock().unwrap().target.clone();
        let active = Job::new(target.clone(), history.clone());
        history.insert(&active, "active.txt".into());
        active.cancel();
        for _ in 0..MAX_HISTORY + 10 {
            let job = Job::new(target.clone(), history.clone());
            history.insert(&job, "done.txt".into());
            job.native.send_replace(Some(true));
            job.finish(Ok(()));
        }
        let entries = history.snapshot();
        assert_eq!(entries.len(), MAX_HISTORY);
        assert_eq!(entries[0].id, active.id);
        assert_eq!(entries[0].state, BrowserDownloadState::Cancelling);
        let before = revision.current();
        history.update(&active.id, |_| {});
        assert_eq!(revision.current(), before);
        let value = serde_json::to_value(&entries[0]).unwrap();
        assert!(value.get("destination").is_none());
        assert!(value.get("path").is_none());
        assert!(value.get("url").is_none());
    }
    #[test]
    fn cleanup_failure_stays_retryable_and_completion_cannot_be_cancelled() {
        let history = DownloadHistory::new(Arc::new(BrowserRevision::default()));
        let job = Job::new(
            control().metadata.lock().unwrap().target.clone(),
            history.clone(),
        );
        history.insert(&job, "file.txt".into());
        job.finish(Err("cleanup uncertain".into()));
        assert!(history.snapshot()[0].can_cancel);
        job.native.send_replace(Some(true));
        job.finish(Ok(()));
        // Model the gap between history publication and done notification.
        job.done.send_replace(None);
        job.cancel();
        assert_eq!(history.snapshot()[0].state, BrowserDownloadState::Completed);
        assert!(!history.snapshot()[0].can_cancel);
    }
    fn control() -> Control {
        Control {
            agent: Mutex::new(None),
            visible: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            locked: Arc::new(AtomicBool::new(false)),
            jobs: Mutex::new(vec![]),
            metadata: Arc::new(Mutex::new(BrowserTabSnapshot {
                target: BrowserTabTarget {
                    tab_id: "tab".into(),
                    runtime_generation: 1,
                    document_generation: 1,
                },
                title: String::new(),
                url: "about:blank".into(),
                lifecycle: nomifun_browser_platform::runtime::BrowserTabLifecycle::Ready,
                can_go_back: false,
                can_go_forward: false,
                blocked_permissions: vec![],
                permission_requests: vec![],
                script_dialog: None,
                diagnostics: Default::default(),
            })),
        }
    }
    #[test]
    fn save_admission_requires_live_visible_idle_browser() {
        let control = control();
        for visible in [false, true] {
            for closed in [false, true] {
                for locked in [false, true] {
                    control.visible.store(visible, Ordering::Release);
                    control.closed.store(closed, Ordering::Release);
                    control.locked.store(locked, Ordering::Release);
                    assert_eq!(control.permitted(), visible && !closed && !locked);
                }
            }
        }
    }
    #[test]
    fn run_or_hide_cancels_only_selection_but_close_cancels_all_transfers() {
        let control = control();
        let target = control.metadata.lock().unwrap().target.clone();
        let history = DownloadHistory::new(Arc::new(BrowserRevision::default()));
        let saving = Job::new(target.clone(), history.clone());
        let transfer = Job::new(target, history);
        transfer.started.store(true, Ordering::Release);
        *control.jobs.lock().unwrap() = vec![saving.clone(), transfer.clone()];
        assert_eq!(control.cancel(false).len(), 1);
        assert!(saving.cancel.is_cancelled());
        assert!(!transfer.cancel.is_cancelled());
        assert_eq!(control.cancel(true).len(), 2);
        assert!(transfer.cancel.is_cancelled());
    }
}
