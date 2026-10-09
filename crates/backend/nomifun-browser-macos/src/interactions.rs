//! Public WebKit/AppKit dialogs and user-mediated file transfers.
//!
//! All retained Objective-C objects and completion blocks live in the native
//! page on the main thread. Only bounded, owned snapshots cross to `Page`.
use crate::engine::{NativeDialog, Page};
use block2::{DynBlock, RcBlock};
use nomifun_browser_platform::runtime::{
    BrowserDialogKind, BrowserDownloadSnapshot, BrowserDownloadState,
};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::{
    NSArray, NSData, NSError, NSObject, NSObjectProtocol, NSProgressReporting, NSString, NSTimer,
    NSURL, NSURLResponse,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKFrameInfo, WKOpenPanelParameters, WKWebView,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    ptr,
    rc::{Rc, Weak as RcWeak},
    sync::{Weak, atomic::Ordering},
};

const MAX_DOWNLOADS: usize = 4;
const MAX_DOWNLOAD_HISTORY: usize = 64;

enum DialogCompletion {
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(Bool)>),
    Prompt(RcBlock<dyn Fn(*mut NSString)>),
}

impl DialogCompletion {
    fn finish(self, accept: bool, text: &str) {
        match self {
            Self::Alert(callback) => callback.call(()),
            Self::Confirm(callback) => callback.call((Bool::new(accept),)),
            Self::Prompt(callback) => {
                let text = accept.then(|| NSString::from_str(text));
                callback.call((text
                    .as_ref()
                    .map_or(ptr::null_mut(), |text| Retained::as_ptr(text).cast_mut()),));
            }
        }
    }
}

struct PendingDialog {
    snapshot: NativeDialog,
    completion: DialogCompletion,
}

struct PendingOpenPanel {
    id: String,
    generation: u64,
    panel: Retained<NSOpenPanel>,
    completion: RcBlock<dyn Fn(*mut NSArray<NSURL>)>,
}

struct DownloadJob {
    download: Retained<WKDownload>,
    // WKDownload.delegate is weak, so its owning job retains the delegate.
    _delegate: Retained<DownloadDelegate>,
    snapshot: BrowserDownloadSnapshot,
    destination: Option<PathBuf>,
    panel: Option<Retained<NSSavePanel>>,
    destination_callback: Option<RcBlock<dyn Fn(*mut NSURL)>>,
}

#[derive(Default)]
struct State {
    dialog: Option<PendingDialog>,
    open_panel: Option<PendingOpenPanel>,
    download_directory: Option<PathBuf>,
    downloads: BTreeMap<usize, DownloadJob>,
    history: Vec<BrowserDownloadSnapshot>,
    progress_timer: Option<Retained<NSTimer>>,
    pending_cancellations: BTreeSet<String>,
    download_drain_waiters: Vec<Box<dyn FnOnce()>>,
}

pub(crate) struct NativeInteractions {
    page: Weak<Page>,
    state: RefCell<State>,
}

impl NativeInteractions {
    pub(crate) fn new(page: Weak<Page>) -> Rc<Self> {
        Rc::new(Self {
            page,
            state: RefCell::new(State::default()),
        })
    }

    fn page(&self) -> Option<std::sync::Arc<Page>> {
        self.page
            .upgrade()
            .filter(|page| !page.close_requested.load(Ordering::Acquire))
    }

    fn user_page(&self) -> Option<std::sync::Arc<Page>> {
        self.page()
            .filter(|page| !page.input_locked() && !page.dialog_draining() && page.is_visible())
    }

    fn changed(&self) {
        if let Some(page) = self.page.upgrade() {
            let snapshot = self.user_download_snapshot();
            if snapshot != page.user_download_snapshot() {
                page.update_downloads(snapshot);
            }
        }
    }

    pub(crate) fn alert(
        self: &Rc<Self>,
        _view: &WKWebView,
        frame: &WKFrameInfo,
        message: &NSString,
        completion: &DynBlock<dyn Fn()>,
    ) {
        self.offer_dialog(
            frame,
            BrowserDialogKind::Alert,
            message,
            None,
            DialogCompletion::Alert(completion.copy()),
        );
    }

    pub(crate) fn confirm(
        self: &Rc<Self>,
        _view: &WKWebView,
        frame: &WKFrameInfo,
        message: &NSString,
        completion: &DynBlock<dyn Fn(Bool)>,
    ) {
        self.offer_dialog(
            frame,
            BrowserDialogKind::Confirm,
            message,
            None,
            DialogCompletion::Confirm(completion.copy()),
        );
    }

    pub(crate) fn prompt(
        self: &Rc<Self>,
        _view: &WKWebView,
        frame: &WKFrameInfo,
        message: &NSString,
        default_text: Option<&NSString>,
        completion: &DynBlock<dyn Fn(*mut NSString)>,
    ) {
        self.offer_dialog(
            frame,
            BrowserDialogKind::Prompt,
            message,
            default_text,
            DialogCompletion::Prompt(completion.copy()),
        );
    }

    fn offer_dialog(
        self: &Rc<Self>,
        frame: &WKFrameInfo,
        kind: BrowserDialogKind,
        message: &NSString,
        default_text: Option<&NSString>,
        completion: DialogCompletion,
    ) {
        let Some(page) = self.page() else {
            completion.finish(false, "");
            return;
        };
        if self.state.borrow().dialog.is_some() || page.dialog_draining() || !page.is_visible() {
            completion.finish(false, "");
            return;
        }
        let (message, message_truncated) = bounded(&message.to_string(), 4096);
        let (default_text, default_truncated) = bounded(
            &default_text.map(ToString::to_string).unwrap_or_default(),
            2048,
        );
        let origin = unsafe { frame.request().URL() }
            .and_then(|value| value.absoluteString())
            .and_then(|value| url::Url::parse(&value.to_string()).ok())
            .filter(|value| matches!(value.scheme(), "http" | "https"))
            .map(|value| value.origin().ascii_serialization())
            .unwrap_or_else(|| "Website".into());
        let snapshot = NativeDialog {
            request_id: uuid::Uuid::now_v7().to_string(),
            document_generation: page.snapshot().document_generation,
            kind,
            message,
            default_text,
            origin,
            text_truncated: message_truncated || default_truncated,
        };
        self.state.borrow_mut().dialog = Some(PendingDialog {
            snapshot: snapshot.clone(),
            completion,
        });
        // Humans and Agents share the renderer's existing WebsiteDialog.
        // Its overlay temporarily hides this native child; the retained WK
        // completion remains pending until the explicit reply/Stop/navigation.
        page.changed(|state| state.dialog = Some(snapshot));
    }

    pub(crate) fn reply_dialog(
        &self,
        id: &str,
        generation: u64,
        accept: bool,
        text: &str,
    ) -> Result<(), String> {
        let page = self
            .page()
            .ok_or_else(|| "WK dialog page is closed".to_owned())?;
        let pending = {
            let mut state = self.state.borrow_mut();
            let pending = state
                .dialog
                .as_ref()
                .ok_or_else(|| "WK dialog is stale".to_owned())?;
            if pending.snapshot.request_id != id
                || pending.snapshot.document_generation != generation
                || page.snapshot().document_generation != generation
            {
                return Err("WK dialog is stale".into());
            }
            state.dialog.take().expect("checked pending dialog")
        };
        page.changed(|state| state.dialog = None);
        pending.completion.finish(accept, text);
        Ok(())
    }

    /// Call before navigation, close, renderer loss, and an input-owner change.
    /// Completion ownership is removed first, so a late sheet callback is inert.
    pub(crate) fn drain_dialogs(&self) {
        let (dialog, panel) = {
            let mut state = self.state.borrow_mut();
            (state.dialog.take(), state.open_panel.take())
        };
        if let Some(page) = self.page.upgrade() {
            page.changed(|state| state.dialog = None);
        }
        if let Some(dialog) = dialog {
            dialog.completion.finish(false, "");
        }
        if let Some(panel) = panel {
            unsafe {
                panel.panel.cancel(None);
            }
            panel.completion.call((ptr::null_mut(),));
        }
    }

    pub(crate) fn open_panel(
        self: &Rc<Self>,
        view: &WKWebView,
        parameters: &WKOpenPanelParameters,
        completion: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
    ) {
        let Some(page) = self.user_page() else {
            completion.call((ptr::null_mut(),));
            return;
        };
        let Some(window) = view.window() else {
            completion.call((ptr::null_mut(),));
            return;
        };
        if self.state.borrow().open_panel.is_some() {
            completion.call((ptr::null_mut(),));
            return;
        }
        let mtm = MainThreadMarker::new().expect("WK file chooser must run on main thread");
        let panel = NSOpenPanel::openPanel(mtm);
        unsafe {
            panel.setAllowsMultipleSelection(parameters.allowsMultipleSelection());
            panel.setCanChooseDirectories(parameters.allowsDirectories());
        }
        panel.setCanChooseFiles(true);
        panel.setResolvesAliases(true);
        // WKOpenPanelParameters exposes multiple/directory selection but no
        // public accept-filter API; WebKit remains responsible for the control.
        let id = uuid::Uuid::now_v7().to_string();
        self.state.borrow_mut().open_panel = Some(PendingOpenPanel {
            id: id.clone(),
            generation: page.snapshot().document_generation,
            panel: panel.clone(),
            completion: completion.copy(),
        });
        let weak = Rc::downgrade(self);
        let finished = RcBlock::new(move |response| {
            let Some(owner) = weak.upgrade() else {
                return;
            };
            let pending = {
                let mut state = owner.state.borrow_mut();
                if state
                    .open_panel
                    .as_ref()
                    .is_none_or(|pending| pending.id != id)
                {
                    return;
                }
                state
                    .open_panel
                    .take()
                    .expect("checked pending file chooser")
            };
            let valid = response == NSModalResponseOK
                && owner
                    .user_page()
                    .is_some_and(|page| page.snapshot().document_generation == pending.generation);
            let urls = valid.then(|| pending.panel.URLs());
            pending.completion.call((urls
                .as_ref()
                .map_or(ptr::null_mut(), |urls| Retained::as_ptr(urls).cast_mut()),));
        });
        panel.beginSheetModalForWindow_completionHandler(&window, &finished);
    }

    pub(crate) fn configure_user_downloads(&self, directory: PathBuf) -> Result<(), String> {
        if !directory.is_absolute() {
            return Err("WK user download directory must be absolute".into());
        }
        std::fs::create_dir_all(&directory)
            .map_err(|_| "WK user download directory cannot be created")?;
        let directory = directory
            .canonicalize()
            .map_err(|_| "WK user download directory cannot be resolved")?;
        self.state.borrow_mut().download_directory = Some(directory);
        Ok(())
    }

    /// A download is admitted only while a human owns the visible page. Agent
    /// automated download is an explicitly unsupported capability in this host.
    pub(crate) fn attach_download(self: &Rc<Self>, download: &WKDownload) {
        let Some(page) = self.user_page() else {
            unsafe {
                download.cancel(None);
            }
            return;
        };
        if self.state.borrow().downloads.len() >= MAX_DOWNLOADS {
            unsafe {
                download.cancel(None);
            }
            return;
        }
        let key = download as *const WKDownload as usize;
        let mtm = MainThreadMarker::new().expect("WK download must run on main thread");
        let id = uuid::Uuid::now_v7().to_string();
        let delegate = DownloadDelegate::new(Rc::downgrade(self), id.clone(), mtm);
        let snapshot = BrowserDownloadSnapshot {
            id,
            tab_id: format!("browser-{}", page.id()),
            filename: String::new(),
            state: BrowserDownloadState::Choosing,
            received_bytes: 0,
            total_bytes: None,
            can_cancel: true,
        };
        self.state.borrow_mut().downloads.insert(
            key,
            DownloadJob {
                download: download.retain(),
                _delegate: delegate.clone(),
                snapshot,
                destination: None,
                panel: None,
                destination_callback: None,
            },
        );
        unsafe {
            download.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        self.start_progress_timer(page.id());
        self.changed();
    }

    fn start_progress_timer(&self, id: uuid::Uuid) {
        if self.state.borrow().progress_timer.is_some() {
            return;
        }
        // The timer is created and invalidated on the existing AppKit run loop.
        // Its sendable block captures only an ID, never Rc or native objects.
        // Native ownership is resolved again on the main thread at each tick.
        let tick = RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            let Some(_) = MainThreadMarker::new() else {
                unsafe {
                    timer.as_ref().invalidate();
                }
                return;
            };
            if let Some(native) = crate::engine::native_page(id) {
                native.interactions.changed();
            } else {
                unsafe {
                    timer.as_ref().invalidate();
                }
            }
        });
        let timer =
            unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &tick) };
        self.state.borrow_mut().progress_timer = Some(timer);
    }

    fn decide_destination(
        self: &Rc<Self>,
        download: &WKDownload,
        id: &str,
        response: &NSURLResponse,
        suggested: &NSString,
        completion: &DynBlock<dyn Fn(*mut NSURL)>,
    ) {
        let key = download as *const WKDownload as usize;
        let filename = suggested.to_string();
        if !self.matches_download(key, id) {
            completion.call((ptr::null_mut(),));
            return;
        }
        if self.user_page().is_none() || !safe_filename(&filename) {
            completion.call((ptr::null_mut(),));
            self.finish_download(key, id, BrowserDownloadState::Cancelled);
            return;
        }
        let (directory, id) = {
            let mut state = self.state.borrow_mut();
            let directory = state.download_directory.clone();
            let Some(job) = state.downloads.get_mut(&key) else {
                drop(state);
                completion.call((ptr::null_mut(),));
                return;
            };
            job.snapshot.filename = filename.clone();
            job.snapshot.total_bytes = (response.expectedContentLength() >= 0)
                .then_some(response.expectedContentLength() as u64);
            job.destination_callback = Some(completion.copy());
            (directory, job.snapshot.id.clone())
        };
        if let Some(directory) = directory {
            self.select_destination(
                key,
                &id,
                Some(directory.join(unique_download_name(&id, &filename))),
            );
            return;
        }
        let window = unsafe { download.webView() }.and_then(|view| view.window());
        let Some(window) = window else {
            self.select_destination(key, &id, None);
            return;
        };
        let panel = NSSavePanel::savePanel(
            MainThreadMarker::new().expect("WK download chooser must run on main thread"),
        );
        panel.setNameFieldStringValue(&NSString::from_str(&filename));
        panel.setCanCreateDirectories(true);
        if let Some(job) = self.state.borrow_mut().downloads.get_mut(&key) {
            job.panel = Some(panel.clone());
        }
        let weak = Rc::downgrade(self);
        let finished = RcBlock::new(move |response| {
            let Some(owner) = weak.upgrade() else {
                return;
            };
            if !owner.matches_download(key, &id) {
                return;
            }
            let destination = if response == NSModalResponseOK && owner.user_page().is_some() {
                owner
                    .state
                    .borrow()
                    .downloads
                    .get(&key)
                    .and_then(|job| job.panel.as_ref())
                    .and_then(|panel| panel.URL())
                    .and_then(|url| url.path())
                    .map(|path| PathBuf::from(path.to_string()))
            } else {
                None
            };
            owner.select_destination(key, &id, destination);
        });
        panel.beginSheetModalForWindow_completionHandler(&window, &finished);
        self.changed();
    }

    fn matches_download(&self, key: usize, id: &str) -> bool {
        self.state
            .borrow()
            .downloads
            .get(&key)
            .is_some_and(|job| job.snapshot.id == id)
    }

    fn select_destination(&self, key: usize, id: &str, destination: Option<PathBuf>) {
        // WKDownload requires a nonexistent destination. Never delete an
        // existing user file merely because a website suggested its name.
        let destination = destination
            .filter(|path| path.is_absolute() && !path.exists() && path.to_str().is_some());
        let callback = {
            let mut state = self.state.borrow_mut();
            let Some(job) = state.downloads.get_mut(&key) else {
                return;
            };
            if job.snapshot.id != id {
                return;
            }
            job.panel = None;
            job.destination = destination.clone();
            if let Some(path) = &destination {
                job.snapshot.state = BrowserDownloadState::InProgress;
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    job.snapshot.filename = name.to_owned();
                }
            }
            job.destination_callback.take()
        };
        let url = destination.as_ref().map(|path| {
            NSURL::fileURLWithPath(&NSString::from_str(
                path.to_str().expect("validated UTF-8 path"),
            ))
        });
        if let Some(callback) = callback {
            callback.call((url
                .as_ref()
                .map_or(ptr::null_mut(), |url| Retained::as_ptr(url).cast_mut()),));
        }
        if destination.is_none() {
            self.finish_download(key, id, BrowserDownloadState::Cancelled);
        } else {
            self.changed();
        }
    }

    pub(crate) fn user_download_snapshot(&self) -> Vec<BrowserDownloadSnapshot> {
        let state = self.state.borrow();
        let mut snapshots = state.history.clone();
        snapshots.extend(state.downloads.values().map(|job| {
            let mut snapshot = job.snapshot.clone();
            let progress = job.download.progress();
            snapshot.received_bytes = progress.completedUnitCount().max(0) as u64;
            if progress.totalUnitCount() > 0 {
                snapshot.total_bytes = Some(progress.totalUnitCount() as u64);
            }
            snapshot
        }));
        snapshots
    }

    pub(crate) fn cancel_user_download(self: &Rc<Self>, id: &str) -> Result<(), String> {
        let key = self
            .state
            .borrow()
            .downloads
            .iter()
            .find_map(|(key, job)| (job.snapshot.id == id).then_some(*key))
            .ok_or_else(|| "WK user download is stale".to_owned())?;
        self.cancel_download_key(key);
        Ok(())
    }

    pub(crate) fn cancel_user_downloads(self: &Rc<Self>) {
        let keys = self
            .state
            .borrow()
            .downloads
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            self.cancel_download_key(key);
        }
    }

    /// An input-owner or visible-tab change revokes outstanding user choices,
    /// while a previously admitted native download remains independently owned.
    pub(crate) fn cancel_user_panels(self: &Rc<Self>) {
        let panel = self.state.borrow_mut().open_panel.take();
        if let Some(panel) = panel {
            unsafe {
                panel.panel.cancel(None);
            }
            panel.completion.call((ptr::null_mut(),));
        }
        let keys = self
            .state
            .borrow()
            .downloads
            .iter()
            .filter_map(|(key, job)| job.destination.is_none().then_some(*key))
            .collect::<Vec<_>>();
        for key in keys {
            self.cancel_download_key(key);
        }
    }

    /// Page close waits for native cancellation acknowledgement, not merely for
    /// the UI to hide or a download's delegate to report an early failure.
    pub(crate) fn cancel_user_downloads_with_completion(
        self: &Rc<Self>,
        completion: Box<dyn FnOnce()>,
    ) {
        self.state
            .borrow_mut()
            .download_drain_waiters
            .push(completion);
        self.cancel_user_downloads();
        self.settle_download_drain();
    }

    fn settle_download_drain(&self) {
        let completions = {
            let mut state = self.state.borrow_mut();
            if !state.downloads.is_empty() || !state.pending_cancellations.is_empty() {
                return;
            }
            std::mem::take(&mut state.download_drain_waiters)
        };
        for completion in completions {
            completion();
        }
    }

    fn cancel_download_key(self: &Rc<Self>, key: usize) {
        let (download, id, panel, callback) = {
            let mut state = self.state.borrow_mut();
            let Some(job) = state.downloads.get_mut(&key) else {
                return;
            };
            if job.snapshot.state == BrowserDownloadState::Cancelling {
                return;
            }
            job.snapshot.state = BrowserDownloadState::Cancelling;
            job.snapshot.can_cancel = false;
            let pending = (
                job.download.clone(),
                job.snapshot.id.clone(),
                job.panel.take(),
                job.destination_callback.take(),
            );
            state.pending_cancellations.insert(pending.1.clone());
            pending
        };
        if let Some(panel) = panel {
            unsafe {
                panel.cancel(None);
            }
        }
        if let Some(callback) = callback {
            callback.call((ptr::null_mut(),));
        }
        // WKDownload retains its cancellation completion until acknowledgement.
        // Keep this owner alive even after NativePage leaves the TLS registry.
        let owner = self.clone();
        let cancelled = RcBlock::new(move |_resume_data: *mut NSData| {
            owner.finish_download(key, &id, BrowserDownloadState::Cancelled);
            owner.state.borrow_mut().pending_cancellations.remove(&id);
            owner.settle_download_drain();
        });
        unsafe {
            download.cancel(Some(&cancelled));
        }
        self.changed();
    }

    fn finish_download(&self, key: usize, id: &str, terminal: BrowserDownloadState) {
        if !self.matches_download(key, id) {
            return;
        }
        let Some(mut job) = self.state.borrow_mut().downloads.remove(&key) else {
            return;
        };
        let progress = job.download.progress();
        job.snapshot.received_bytes = progress.completedUnitCount().max(0) as u64;
        if terminal == BrowserDownloadState::Completed {
            if let Some(metadata) = job
                .destination
                .as_ref()
                .and_then(|path| std::fs::metadata(path).ok())
            {
                job.snapshot.received_bytes = metadata.len();
                job.snapshot.total_bytes = Some(metadata.len());
            }
        }
        job.snapshot.state = if job.snapshot.state == BrowserDownloadState::Cancelling {
            BrowserDownloadState::Cancelled
        } else {
            terminal
        };
        job.snapshot.can_cancel = false;
        if let Some(panel) = job.panel.take() {
            unsafe {
                panel.cancel(None);
            }
        }
        if let Some(callback) = job.destination_callback.take() {
            callback.call((ptr::null_mut(),));
        }
        unsafe {
            job.download.setDelegate(None);
        }
        {
            let mut state = self.state.borrow_mut();
            if state.history.len() >= MAX_DOWNLOAD_HISTORY {
                state.history.remove(0);
            }
            state.history.push(job.snapshot);
            if state.downloads.is_empty() {
                if let Some(timer) = state.progress_timer.take() {
                    timer.invalidate();
                }
            }
        }
        self.changed();
        self.settle_download_drain();
    }
}

impl Drop for NativeInteractions {
    fn drop(&mut self) {
        self.drain_dialogs();
        if let Some(timer) = self.state.get_mut().progress_timer.take() {
            timer.invalidate();
        }
        let jobs = std::mem::take(&mut self.state.get_mut().downloads);
        for mut job in jobs.into_values() {
            if let Some(panel) = job.panel.take() {
                unsafe {
                    panel.cancel(None);
                }
            }
            if let Some(callback) = job.destination_callback.take() {
                callback.call((ptr::null_mut(),));
            }
            unsafe {
                job.download.setDelegate(None);
                job.download.cancel(None);
            }
        }
    }
}

fn bounded(value: &str, limit: usize) -> (String, bool) {
    let bounded = value.chars().take(limit).collect::<String>();
    let truncated = bounded.len() != value.len();
    (bounded, truncated)
}

fn safe_filename(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 240
        && !value
            .chars()
            .any(|value| matches!(value, '/' | '\\' | ':' | '\0') || value.is_control())
}

fn unique_download_name(id: &str, filename: &str) -> String {
    let mut name = format!("NomiFun-{id}-");
    if name.len() + filename.len() <= 255 {
        name.push_str(filename);
        return name;
    }
    let extension = std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.len() <= 32)
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let stem = filename.strip_suffix(&extension).unwrap_or(filename);
    for character in stem.chars() {
        if name.len() + character.len_utf8() + extension.len() > 255 {
            break;
        }
        name.push(character);
    }
    name.push_str(&extension);
    name
}

#[derive(Default)]
struct DownloadDelegateIvars {
    owner: RcWeak<NativeInteractions>,
    id: String,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NomiFunWKDownloadDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DownloadDelegateIvars]
    struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    unsafe impl WKDownloadDelegate for DownloadDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        unsafe fn destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            suggested: &NSString,
            completion: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            if let Some(owner) = self.ivars().owner.upgrade() {
                owner.decide_destination(
                    download,
                    &self.ivars().id,
                    response,
                    suggested,
                    completion,
                );
            } else {
                completion.call((ptr::null_mut(),));
            }
        }

        #[unsafe(method(downloadDidFinish:))]
        unsafe fn finished(&self, download: &WKDownload) {
            if let Some(owner) = self.ivars().owner.upgrade() {
                owner.finish_download(
                    download as *const WKDownload as usize,
                    &self.ivars().id,
                    BrowserDownloadState::Completed,
                );
            }
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        unsafe fn failed(
            &self,
            download: &WKDownload,
            _error: &NSError,
            _resume_data: Option<&NSData>,
        ) {
            if let Some(owner) = self.ivars().owner.upgrade() {
                owner.finish_download(
                    download as *const WKDownload as usize,
                    &self.ivars().id,
                    BrowserDownloadState::Failed,
                );
            }
        }
    }
);

impl DownloadDelegate {
    fn new(owner: RcWeak<NativeInteractions>, id: String, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DownloadDelegateIvars { owner, id });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bool, BrowserDialogKind, DialogCompletion, NativeDialog, NativeInteractions, PendingDialog,
        RcBlock, bounded, safe_filename, unique_download_name,
    };
    use std::{cell::Cell, rc::Rc, sync::Weak};

    #[test]
    fn cosmetic_surface_hide_keeps_website_dialog_pending() {
        let interactions = NativeInteractions::new(Weak::new());
        let answer = Rc::new(Cell::new(None));
        let completion_answer = answer.clone();
        let completion =
            RcBlock::new(move |value: Bool| completion_answer.set(Some(value.as_bool())));
        interactions.state.borrow_mut().dialog = Some(PendingDialog {
            snapshot: NativeDialog {
                request_id: "dialog".into(),
                document_generation: 1,
                kind: BrowserDialogKind::Confirm,
                message: "Continue?".into(),
                default_text: String::new(),
                origin: "https://fixture.invalid".into(),
                text_truncated: false,
            },
            completion: DialogCompletion::Confirm(completion),
        });
        interactions.cancel_user_panels();
        assert!(interactions.state.borrow().dialog.is_some());
        assert_eq!(answer.get(), None);
        interactions.drain_dialogs();
        assert_eq!(answer.get(), Some(false));
        assert!(interactions.state.borrow().dialog.is_none());
    }

    #[test]
    fn download_drain_waits_for_cancellation_ack_after_job_disappears() {
        let owner = NativeInteractions::new(Weak::new());
        let calls = Rc::new(Cell::new(0));
        let completed = calls.clone();
        owner
            .state
            .borrow_mut()
            .pending_cancellations
            .insert("pending-native-ack".into());
        owner.cancel_user_downloads_with_completion(Box::new(move || {
            completed.set(completed.get() + 1)
        }));
        assert_eq!(
            calls.get(),
            0,
            "an empty download map is not cancellation acknowledgement"
        );
        owner.state.borrow_mut().pending_cancellations.clear();
        owner.settle_download_drain();
        owner.settle_download_drain();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn empty_download_drain_releases_borrow_before_completion() {
        let owner = NativeInteractions::new(Weak::new());
        let reentrant = owner.clone();
        let calls = Rc::new(Cell::new(0));
        let completed = calls.clone();
        owner.cancel_user_downloads_with_completion(Box::new(move || {
            completed.set(completed.get() + 1);
            reentrant.cancel_user_downloads_with_completion(Box::new(move || {
                completed.set(completed.get() + 1)
            }));
        }));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn website_download_names_cannot_escape_the_destination() {
        for name in [
            "",
            ".",
            "..",
            "../secret",
            "folder/file",
            "folder\\file",
            "foo:bar",
            "foo\0bar",
            "line\nfeed",
        ] {
            assert!(!safe_filename(name), "{name:?}");
        }
        assert!(safe_filename("报告 2026.pdf"));
    }

    #[test]
    fn bounded_page_dialog_text_preserves_unicode_boundaries() {
        assert_eq!(bounded("网页提示", 2), ("网页".into(), true));
        assert_eq!(bounded("网页", 2), ("网页".into(), false));
    }

    #[test]
    fn unique_download_filename_fits_the_filesystem_byte_limit() {
        let id = uuid::Uuid::nil().to_string();
        let filename = unique_download_name(&id, &format!("{}.pdf", "文".repeat(80)));
        assert!(filename.len() <= 255);
        assert!(filename.starts_with(&format!("NomiFun-{id}-")));
        assert!(filename.ends_with(".pdf"));
    }
}
