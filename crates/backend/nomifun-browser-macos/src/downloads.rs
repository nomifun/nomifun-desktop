//! One explicit Agent click may admit one CEF download into a host-owned
//! private file. Native terminal proof precedes validation and publication.

use cef::*;
use crate::engine::Page;
use nomifun_browser_platform::{
    downloads::{BrowserDownloadArtifact, PreparedBrowserDownload},
    runtime::{BrowserDownloadSnapshot, BrowserDownloadState, WorkspaceError},
};
use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, AtomicU32, Ordering}}};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
enum NativeState {
    Waiting,
    InProgress,
    Terminal(Result<NativeTerminal, String>),
}

#[derive(Clone)]
struct NativeTerminal {
    filename: String,
    received: u64,
}

pub struct AgentDownloadRequest {
    file: Arc<PreparedBrowserDownload>,
    cancel: CancellationToken,
    accepting: AtomicBool,
    native_id: AtomicU32,
    filename: Mutex<Option<String>>,
    native_path: Mutex<Option<PathBuf>>,
    callback: Mutex<Option<DownloadItemCallback>>,
    state: watch::Sender<NativeState>,
}

pub(crate) struct UserDownloads {
    directory: Option<PathBuf>,
    jobs: BTreeMap<u32, UserDownload>,
    early: BTreeMap<u32, EarlyUserDownload>,
    rejected: BTreeSet<u32>,
    history: Vec<BrowserDownloadSnapshot>,
}

struct EarlyUserDownload {
    callback: Option<DownloadItemCallback>,
}

struct UserDownload {
    id: String,
    callback: Option<DownloadItemCallback>,
    cancelling: bool,
}

impl Default for UserDownloads {
    fn default() -> Self {
        Self {
            directory: None,
            jobs: BTreeMap::new(),
            early: BTreeMap::new(),
            rejected: BTreeSet::new(),
            history: Vec::new(),
        }
    }
}

impl Page {
    pub fn configure_user_downloads(&self, directory: PathBuf) -> Result<(), String> {
        if !directory.is_absolute() {
            return Err("CEF user download directory is invalid".into());
        }
        std::fs::create_dir_all(&directory)
            .map_err(|_| "CEF user download directory cannot be created")?;
        self.user_downloads.lock().unwrap().directory = Some(
            directory
                .canonicalize()
                .map_err(|_| "CEF user download directory cannot be resolved")?,
        );
        Ok(())
    }

    pub fn user_download_snapshot(&self) -> Vec<BrowserDownloadSnapshot> {
        self.user_downloads.lock().unwrap().history.clone()
    }

    pub fn arm_agent_download(
        self: &Arc<Self>,
        file: Arc<PreparedBrowserDownload>,
        cancel: CancellationToken,
    ) -> Result<Arc<AgentDownloadRequest>, WorkspaceError> {
        if !self.input_locked()
            || self.close_requested.load(Ordering::Acquire)
            || cancel.is_cancelled()
        {
            return Err(WorkspaceError::StaleTarget);
        }
        let request = Arc::new(AgentDownloadRequest {
            file,
            cancel: cancel.child_token(),
            accepting: AtomicBool::new(true),
            native_id: AtomicU32::new(0),
            filename: Mutex::new(None),
            native_path: Mutex::new(None),
            callback: Mutex::new(None),
            state: watch::channel(NativeState::Waiting).0,
        });
        let mut slot = self.download.lock().unwrap();
        if slot.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(WorkspaceError::NotActionable);
        }
        *slot = Some(Arc::downgrade(&request));
        Ok(request)
    }

    fn pending_download(&self) -> Option<Arc<AgentDownloadRequest>> {
        self.download.lock().unwrap().as_ref().and_then(Weak::upgrade)
    }

    pub(super) fn can_download(&self, url: Option<&CefString>) -> bool {
        let parsed = url.and_then(|value| url::Url::parse(&value.to_string()).ok());
        let valid_url = parsed.as_ref().is_some_and(|url| matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none());
        let result = if !valid_url {
            false
        } else if let Some(request) = self.pending_download() {
            request.accepting.load(Ordering::Acquire)
                && request.native_id.load(Ordering::Acquire) == 0
                && !request.cancel.is_cancelled()
                && self.input_locked()
        } else {
            let user = self.user_downloads.lock().unwrap();
            !self.input_locked()
                && self.is_visible()
                && user.directory.is_some()
                && user.jobs.len() + user.early.len() < 4
        };
        result
    }

    pub(super) fn begin_download(
        &self,
        item: Option<&mut DownloadItem>,
        suggested_name: Option<&CefString>,
        callback: Option<&mut BeforeDownloadCallback>,
    ) -> bool {
        let Some(request) = self.pending_download() else {
            return self.begin_user_download(item, suggested_name, callback);
        };
        if request.cancel.is_cancelled() || !self.input_locked() {
            return false;
        }
        let Some(item) = item else {
            request.state.send_replace(NativeState::Terminal(Err("CEF download item is unavailable".into())));
            return false;
        };
        let Some(callback) = callback else {
            request.state.send_replace(NativeState::Terminal(Err("CEF download callback is unavailable".into())));
            return false;
        };
        let current_id = request.native_id.load(Ordering::Acquire);
        if current_id == 0 {
            if !request.accepting.swap(false, Ordering::AcqRel)
                || request
                    .native_id
                    .compare_exchange(0, item.id(), Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return false;
            }
        } else if current_id != item.id() {
            return false;
        } else {
            // OnDownloadUpdated is explicitly allowed before OnBeforeDownload.
            // Its first update already claimed this exact native item.
            request.accepting.store(false, Ordering::Release);
        }
        let filename = suggested_name.map(ToString::to_string).unwrap_or_default();
        if !safe_filename(&filename) {
            request.state.send_replace(NativeState::Terminal(Err("CEF download filename is invalid".into())));
            return false;
        }
        // Keep CEF's native target inside the capability-owned private
        // directory while preserving the server filename during its native
        // target handling. Normalize it back to the fixed `payload` name only
        // after CEF releases the writer.
        let private_path = request.file.native_path();
        let Some(private_directory) = private_path.parent() else {
            request.state.send_replace(NativeState::Terminal(Err("CEF download directory is unavailable".into())));
            return false;
        };
        let path = private_directory.join(format!("native-{}-{filename}", uuid::Uuid::now_v7()));
        let Some(path_string) = path.to_str().map(str::to_owned) else {
            request.state.send_replace(NativeState::Terminal(Err("CEF download path must be UTF-8".into())));
            return false;
        };
        *request.filename.lock().unwrap() = Some(filename);
        *request.native_path.lock().unwrap() = Some(path);
        request.state.send_replace(NativeState::InProgress);
        callback.cont(Some(&CefString::from(path_string.as_str())), 0);
        true
    }

    pub(super) fn update_download(
        &self,
        item: Option<&mut DownloadItem>,
        callback: Option<&mut DownloadItemCallback>,
    ) {
        let Some(item) = item else {
            if let Some(request) = self.pending_download() {
                request.state.send_replace(NativeState::Terminal(Err("CEF download update is unavailable".into())));
            }
            return;
        };
        let Some(request) = self.pending_download() else {
            self.update_user_download(item, callback);
            return;
        };
        let mut native_id = request.native_id.load(Ordering::Acquire);
        if native_id == 0 && request.accepting.load(Ordering::Acquire) {
            match request.native_id.compare_exchange(
                0,
                item.id(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    request.accepting.store(false, Ordering::Release);
                    native_id = item.id();
                }
                Err(claimed) => native_id = claimed,
            }
        }
        if item.id() != native_id {
            if let Some(callback) = callback {
                callback.cancel();
            }
            return;
        }
        if let Some(callback) = callback {
            *request.callback.lock().unwrap() = Some(callback.clone());
        }
        let received = item.received_bytes().max(0) as u64;
        let total = (item.total_bytes() >= 0).then_some(item.total_bytes() as u64);
        if let Err(error) = request.file.progress(received, total) {
            if let Some(callback) = request.callback.lock().unwrap().as_ref() { callback.cancel(); }
            request.state.send_replace(NativeState::Terminal(Err(error.code().into())));
            return;
        }
        if request.cancel.is_cancelled() {
            if let Some(callback) = request.callback.lock().unwrap().as_ref() { callback.cancel(); }
        }
        if item.is_complete() != 0 {
            let full_path = CefString::from(&item.full_path()).to_string();
            let expected = request
                .native_path
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|path| std::fs::canonicalize(path).ok());
            let observed = std::fs::canonicalize(std::path::PathBuf::from(full_path));
            let same_path = matches!((expected, observed), (Some(expected), Ok(observed)) if expected == observed);
            if !same_path {
                request.state.send_replace(NativeState::Terminal(Err("CEF download completed at an unexpected path".into())));
            } else {
                let filename = request
                    .filename
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| CefString::from(&item.suggested_file_name()).to_string());
                request.state.send_replace(NativeState::Terminal(Ok(NativeTerminal { filename, received })));
            }
        } else if item.is_canceled() != 0 || item.is_interrupted() != 0 {
            request.state.send_replace(NativeState::Terminal(Err("CEF download did not complete".into())));
        }
    }

    fn begin_user_download(
        &self,
        item: Option<&mut DownloadItem>,
        suggested_name: Option<&CefString>,
        callback: Option<&mut BeforeDownloadCallback>,
    ) -> bool {
        if self.input_locked() || !self.is_visible() {
            if let Some(item) = item {
                self.reject_user_download(item.id());
            }
            return false;
        }
        let Some(item) = item else {
            return false;
        };
        let native_id = item.id();
        let Some(callback) = callback else {
            self.reject_user_download(native_id);
            return false;
        };
        let filename = suggested_name.map(ToString::to_string).unwrap_or_default();
        if !safe_filename(&filename) {
            self.reject_user_download(native_id);
            return false;
        }
        let mut owner = self.user_downloads.lock().unwrap();
        let Some(directory) = owner.directory.clone() else {
            drop(owner);
            self.reject_user_download(native_id);
            return false;
        };
        if !owner.early.contains_key(&native_id)
            && owner.jobs.len() + owner.early.len() >= 4
        {
            drop(owner);
            self.reject_user_download(native_id);
            return false;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let native_name = format!("NomiFun-{id}-{filename}");
        let destination = directory.join(&native_name);
        if destination.exists() {
            drop(owner);
            self.reject_user_download(native_id);
            return false;
        }
        let Some(path) = destination.to_str() else {
            drop(owner);
            self.reject_user_download(native_id);
            return false;
        };
        let tab_id = format!("browser-{}", self.id());
        let early = owner.early.remove(&native_id);
        owner.jobs.insert(native_id, UserDownload {
            id: id.clone(),
            callback: early.and_then(|early| early.callback),
            cancelling: false,
        });
        if owner.history.len() >= 64 {
            if let Some(index) = owner.history.iter().position(|entry| !entry.can_cancel) {
                owner.history.remove(index);
            } else {
                owner.jobs.remove(&native_id);
                owner.rejected.insert(native_id);
                bound_rejected(&mut owner);
                return false;
            }
        }
        owner.history.push(BrowserDownloadSnapshot {
            id,
            tab_id,
            filename: native_name,
            state: BrowserDownloadState::InProgress,
            received_bytes: 0,
            total_bytes: None,
            can_cancel: true,
        });
        drop(owner);
        callback.cont(Some(&CefString::from(path)), 0);
        self.changed(|_| {});
        true
    }

    fn update_user_download(
        &self,
        item: &mut DownloadItem,
        callback: Option<&mut DownloadItemCallback>,
    ) {
        let allow_early = !self.input_locked()
            && self.is_visible()
            && !self.close_requested.load(Ordering::Acquire);
        let mut owner = self.user_downloads.lock().unwrap();
        let native_id = item.id();
        if owner.rejected.contains(&native_id) {
            let terminal = item.is_complete() != 0
                || item.is_canceled() != 0
                || item.is_interrupted() != 0;
            if terminal {
                owner.rejected.remove(&native_id);
            }
            drop(owner);
            if let Some(callback) = callback {
                callback.cancel();
            }
            return;
        }
        if !owner.jobs.contains_key(&native_id) {
            if !allow_early
                || owner.directory.is_none()
                || (!owner.early.contains_key(&native_id)
                    && owner.jobs.len() + owner.early.len() >= 4)
            {
                drop(owner);
                if let Some(callback) = callback {
                    callback.cancel();
                }
                return;
            }
            let early = owner
                .early
                .entry(native_id)
                .or_insert(EarlyUserDownload { callback: None });
            if let Some(callback) = callback {
                early.callback = Some(callback.clone());
            }
            if item.is_complete() != 0
                || item.is_canceled() != 0
                || item.is_interrupted() != 0
            {
                let callback = owner.early.remove(&native_id).and_then(|early| early.callback);
                owner.rejected.insert(native_id);
                bound_rejected(&mut owner);
                drop(owner);
                if let Some(callback) = callback {
                    callback.cancel();
                }
            }
            return;
        }
        let job = owner.jobs.get_mut(&native_id).unwrap();
        if let Some(callback) = callback {
            job.callback = Some(callback.clone());
        }
        if job.cancelling {
            if let Some(callback) = job.callback.as_ref() { callback.cancel(); }
        }
        let job_id = job.id.clone();
        let received = item.received_bytes().max(0) as u64;
        let total = (item.total_bytes() >= 0).then_some(item.total_bytes() as u64);
        let terminal = if item.is_complete() != 0 {
            Some(BrowserDownloadState::Completed)
        } else if item.is_canceled() != 0 {
            Some(BrowserDownloadState::Cancelled)
        } else if item.is_interrupted() != 0 {
            Some(BrowserDownloadState::Failed)
        } else {
            None
        };
        let cancelling = job.cancelling;
        if let Some(entry) = owner.history.iter_mut().find(|entry| entry.id == job_id) {
            entry.received_bytes = received;
            entry.total_bytes = total;
            if let Some(terminal) = terminal {
                entry.state = terminal;
                entry.can_cancel = false;
            } else if cancelling {
                entry.state = BrowserDownloadState::Cancelling;
                entry.can_cancel = false;
            }
        }
        if terminal.is_some() {
            owner.jobs.remove(&native_id);
        }
        drop(owner);
        self.changed(|_| {});
    }

    pub fn cancel_user_download(&self, id: &str) -> Result<(), String> {
        let mut owner = self.user_downloads.lock().unwrap();
        let native_id = owner.jobs.iter().find_map(|(native_id, job)| (job.id == id).then_some(*native_id))
            .ok_or_else(|| "CEF user download is stale".to_owned())?;
        let job = owner.jobs.get_mut(&native_id).unwrap();
        job.cancelling = true;
        if let Some(callback) = job.callback.as_ref() { callback.cancel(); }
        if let Some(entry) = owner.history.iter_mut().find(|entry| entry.id == id) {
            entry.state = BrowserDownloadState::Cancelling;
            entry.can_cancel = false;
        }
        drop(owner);
        self.changed(|_| {});
        Ok(())
    }

    pub fn cancel_user_downloads(&self) {
        let mut owner = self.user_downloads.lock().unwrap();
        for job in owner.jobs.values_mut() {
            job.cancelling = true;
            if let Some(callback) = job.callback.as_ref() { callback.cancel(); }
        }
        for entry in &mut owner.history {
            if entry.can_cancel {
                entry.state = BrowserDownloadState::Cancelling;
                entry.can_cancel = false;
            }
        }
        let early = std::mem::take(&mut owner.early);
        owner.rejected.extend(early.keys().copied());
        bound_rejected(&mut owner);
        drop(owner);
        for callback in early.into_values().filter_map(|early| early.callback) {
            callback.cancel();
        }
        self.changed(|_| {});
    }

    pub(crate) fn cancel_surface_downloads(&self) {
        self.cancel_user_downloads();
        if let Some(request) = self.pending_download() {
            request.accepting.store(false, Ordering::Release);
            request.cancel.cancel();
            if let Some(callback) = request.callback.lock().unwrap().as_ref() {
                callback.cancel();
            }
            request.state.send_replace(NativeState::Terminal(Err(
                "CEF download lost its visible surface".into(),
            )));
        }
    }

    fn reject_user_download(&self, native_id: u32) {
        let callback = {
            let mut owner = self.user_downloads.lock().unwrap();
            let callback = owner.early.remove(&native_id).and_then(|early| early.callback);
            owner.rejected.insert(native_id);
            bound_rejected(&mut owner);
            callback
        };
        if let Some(callback) = callback {
            callback.cancel();
        }
    }

    pub async fn finish_agent_download(
        &self,
        request: Arc<AgentDownloadRequest>,
        action: Result<(), WorkspaceError>,
    ) -> Result<BrowserDownloadArtifact, WorkspaceError> {
        if action.is_err() {
            request.accepting.store(false, Ordering::Release);
            request.cancel.cancel();
        }
        let mut state = request.state.subscribe();
        let claimed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if !matches!(*state.borrow_and_update(), NativeState::Waiting) { return Ok::<_, ()>(()); }
                state.changed().await.map_err(|_| ())?;
            }
        }).await;
        request.accepting.store(false, Ordering::Release);
        if !matches!(claimed, Ok(Ok(()))) {
            request.cancel.cancel();
            if let Some(callback) = request.callback.lock().unwrap().as_ref() {
                callback.cancel();
            }
            self.clear_download(&request);
            return Err(WorkspaceError::DownloadDenied);
        }
        if let Err(error) = action {
            if let Some(callback) = request.callback.lock().unwrap().as_ref() {
                callback.cancel();
            }
            let settled = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    if matches!(*state.borrow_and_update(), NativeState::Terminal(_)) {
                        return;
                    }
                    if state.changed().await.is_err() {
                        return;
                    }
                }
            })
            .await;
            self.clear_download(&request);
            if settled.is_err() {
                return Err(WorkspaceError::NativeCommandFailed);
            }
            return Err(error);
        }
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                if let NativeState::Terminal(result) = state.borrow_and_update().clone() { return result; }
                state.changed().await.map_err(|_| "CEF download owner closed".to_owned())?;
            }
        }).await;
        if terminal.is_err() {
            request.cancel.cancel();
            if let Some(callback) = request.callback.lock().unwrap().as_ref() { callback.cancel(); }
        }
        let terminal = match terminal {
            Ok(terminal) => terminal,
            Err(_) => {
                self.clear_download(&request);
                return Err(WorkspaceError::NativeCommandFailed);
            }
        };
        self.clear_download(&request);
        if request.cancel.is_cancelled() {
            return Err(WorkspaceError::ActionInterrupted);
        }
        let terminal = terminal.map_err(|_| WorkspaceError::DownloadDenied)?;
        normalize_agent_download_path(&request)?;
        request.file.progress(terminal.received, Some(terminal.received))?;
        let result = request.file.publish(
            terminal.filename,
            |name, bytes| {
                !nomi_browser_engine::download::is_executable_denylist(name)
                    && !nomi_browser_engine::download::sniff_is_executable(bytes)
            },
            request.cancel.clone(),
        ).await;
        result
    }

    fn clear_download(&self, request: &Arc<AgentDownloadRequest>) {
        let mut slot = self.download.lock().unwrap();
        if slot.as_ref().and_then(Weak::upgrade).is_some_and(|current| Arc::ptr_eq(&current, request)) {
            slot.take();
        }
    }

    pub async fn cancel_agent_download(&self) -> Result<(), String> {
        let Some(request) = self.pending_download() else { return Ok(()); };
        request.accepting.store(false, Ordering::Release);
        request.cancel.cancel();
        if let Some(callback) = request.callback.lock().unwrap().as_ref() { callback.cancel(); }
        let mut state = request.state.subscribe();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if matches!(*state.borrow_and_update(), NativeState::Terminal(_)) { break; }
                if state.changed().await.is_err() { break; }
            }
        }).await;
        self.clear_download(&request);
        Ok(())
    }
}

fn bound_rejected(owner: &mut UserDownloads) {
    while owner.rejected.len() > 64 {
        let Some(first) = owner.rejected.first().copied() else { break; };
        owner.rejected.remove(&first);
    }
}

fn normalize_agent_download_path(request: &AgentDownloadRequest) -> Result<(), WorkspaceError> {
    let source = request
        .native_path
        .lock()
        .unwrap()
        .take()
        .ok_or(WorkspaceError::DownloadDenied)?;
    let destination = request.file.native_path();
    let source_metadata = std::fs::symlink_metadata(&source)
        .map_err(|_| WorkspaceError::DownloadDenied)?;
    if !source_metadata.file_type().is_file() || destination.exists() {
        return Err(WorkspaceError::DownloadDenied);
    }
    let source_parent = source
        .parent()
        .and_then(|path| path.canonicalize().ok())
        .ok_or(WorkspaceError::DownloadDenied)?;
    let destination_parent = destination
        .parent()
        .and_then(|path| path.canonicalize().ok())
        .ok_or(WorkspaceError::DownloadDenied)?;
    if source_parent != destination_parent {
        return Err(WorkspaceError::DownloadDenied);
    }
    std::fs::rename(source, destination).map_err(|_| WorkspaceError::DownloadDenied)
}

fn safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.encode_utf16().count() <= 160
        && !name.ends_with([' ', '.'])
        && !name.chars().any(|character| character.is_control() || "/\\:*?\"<>|".contains(character))
        && !matches!(name, "." | "..")
}
