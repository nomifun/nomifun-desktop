use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use nomifun_api_types::{PreviewState, PreviewStatusEvent, WebSocketMessage};
use nomifun_realtime::EventBroadcaster;
use nomifun_runtime::Builder as CmdBuilder;
use tokio::sync::Mutex;

use crate::error::OfficeError;
use crate::port::{allocate_port, is_port_listening};
use crate::types::DocType;

const POLL_INTERVAL_MS: u64 = 100;
const POLL_MAX_ATTEMPTS: u32 = 150;
const STOP_DELAY_MS: u64 = 500;
const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

// ---------------------------------------------------------------------------
// ProcessSpawner trait — abstraction for child process management
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait ProcessSpawner: Send + Sync {
    async fn spawn_officecli(
        &self,
        file_path: &str,
        port: u16,
        doc_type: DocType,
    ) -> Result<Box<dyn ProcessHandle>, OfficeError>;

    async fn install_officecli(&self) -> Result<(), OfficeError>;

    async fn is_officecli_installed(&self) -> bool;

    async fn check_update(&self, doc_type: DocType) -> Result<(), OfficeError>;
}

pub trait ProcessHandle: Send + Sync {
    fn kill(&self);
    fn is_alive(&self) -> bool;
}

// ---------------------------------------------------------------------------
// WatchSession — per-file preview session
// ---------------------------------------------------------------------------

struct WatchSession {
    port: u16,
    process: Box<dyn ProcessHandle>,
    file_path: String,
    doc_type: DocType,
    aborted: bool,
}

// ---------------------------------------------------------------------------
// OfficecliWatchManager
// ---------------------------------------------------------------------------

pub struct OfficecliWatchManager {
    sessions: DashMap<String, WatchSession>,
    spawner: Arc<dyn ProcessSpawner>,
    broadcaster: Arc<dyn EventBroadcaster>,
    last_version_check: Mutex<Option<std::time::Instant>>,
}

impl OfficecliWatchManager {
    pub fn new(spawner: Arc<dyn ProcessSpawner>, broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self {
            sessions: DashMap::new(),
            spawner,
            broadcaster,
            last_version_check: Mutex::new(None),
        }
    }

    pub async fn start(&self, file_path: &str, doc_type: DocType) -> Result<u16, OfficeError> {
        let resolved = resolve_path(file_path)?;
        let key = session_key(&resolved, doc_type);

        if let Some(entry) = self.sessions.get(&key) {
            if !entry.aborted && entry.process.is_alive() {
                return Ok(entry.port);
            }
            drop(entry);
            self.sessions.remove(&key);
        }

        self.broadcast_status(doc_type, PreviewState::Starting, None);

        let result = self.try_start(&resolved, doc_type).await;

        match &result {
            Ok(port) => {
                self.broadcast_status(doc_type, PreviewState::Ready, None);
                if doc_type == DocType::Ppt {
                    self.maybe_check_update(doc_type).await;
                }
                Ok(*port)
            }
            Err(e) => {
                self.broadcast_status(doc_type, PreviewState::Error, Some(e.to_string()));
                Err(match e {
                    OfficeError::OfficecliNotFound => OfficeError::OfficecliNotFound,
                    OfficeError::InstallFailed(m) => OfficeError::InstallFailed(m.clone()),
                    OfficeError::StartFailed(m) => OfficeError::StartFailed(m.clone()),
                    OfficeError::PortTimeout(m) => OfficeError::PortTimeout(m.clone()),
                    OfficeError::Io(io) => OfficeError::StartFailed(format!("IO error: {io}")),
                    OfficeError::Snapshot(m) => OfficeError::StartFailed(m.clone()),
                    OfficeError::Json(e) => OfficeError::StartFailed(format!("JSON error: {e}")),
                    OfficeError::Conversion(m) => OfficeError::StartFailed(m.clone()),
                    OfficeError::ToolNotFound(m) => OfficeError::StartFailed(m.clone()),
                })
            }
        }
    }

    async fn try_start(&self, resolved: &str, doc_type: DocType) -> Result<u16, OfficeError> {
        let port = allocate_port()?;

        let spawn_result = self.spawner.spawn_officecli(resolved, port, doc_type).await;

        let process = match spawn_result {
            Ok(p) => p,
            Err(OfficeError::OfficecliNotFound) => {
                self.broadcast_status(doc_type, PreviewState::Installing, None);
                self.spawner.install_officecli().await?;
                self.spawner.spawn_officecli(resolved, port, doc_type).await?
            }
            Err(e) => return Err(e),
        };

        self.poll_port_ready(port, resolved).await?;

        let key = session_key(resolved, doc_type);
        self.sessions.insert(
            key,
            WatchSession {
                port,
                process,
                file_path: resolved.to_owned(),
                doc_type,
                aborted: false,
            },
        );

        Ok(port)
    }

    async fn poll_port_ready(&self, port: u16, file_path: &str) -> Result<(), OfficeError> {
        for _ in 0..POLL_MAX_ATTEMPTS {
            if is_port_listening(port).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
        Err(OfficeError::PortTimeout(file_path.to_owned()))
    }

    pub async fn stop(&self, file_path: &str, doc_type: DocType) {
        let resolved = match resolve_path(file_path) {
            Ok(p) => p,
            Err(_) => return,
        };
        let key = session_key(&resolved, doc_type);

        tokio::time::sleep(Duration::from_millis(STOP_DELAY_MS)).await;

        if let Some((_, session)) = self.sessions.remove(&key) {
            session.process.kill();
        }
    }

    pub fn stop_all(&self) {
        for entry in self.sessions.iter() {
            tracing::debug!(
                file_path = %entry.value().file_path,
                doc_type = %entry.value().doc_type,
                "stopping preview session"
            );
            entry.value().process.kill();
        }
        self.sessions.clear();
    }

    pub fn is_active_port(&self, port: u16, doc_type: DocType) -> bool {
        self.sessions
            .iter()
            .any(|entry| entry.port == port && entry.doc_type == doc_type)
    }

    pub fn is_active_watch_port(&self, port: u16) -> bool {
        self.sessions
            .iter()
            .any(|entry| entry.port == port && matches!(entry.doc_type, DocType::Word | DocType::Excel))
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    async fn maybe_check_update(&self, doc_type: DocType) {
        let mut last = self.last_version_check.lock().await;
        let should_check = match *last {
            Some(t) => t.elapsed() >= VERSION_CHECK_INTERVAL,
            None => true,
        };
        if should_check {
            *last = Some(std::time::Instant::now());
            drop(last);
            let spawner = Arc::clone(&self.spawner);
            tokio::spawn(async move {
                if let Err(e) = spawner.check_update(doc_type).await {
                    tracing::warn!("officecli version check failed: {e}");
                }
            });
        }
    }

    fn broadcast_status(&self, doc_type: DocType, state: PreviewState, message: Option<String>) {
        let event_name = format!("{}.status", doc_type.event_prefix());
        let payload = PreviewStatusEvent { state, message };
        let data = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("failed to serialize preview status: {e}");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, data));
    }
}

impl Drop for OfficecliWatchManager {
    fn drop(&mut self) {
        for entry in self.sessions.iter() {
            entry.value().process.kill();
        }
        self.sessions.clear();
    }
}

// ---------------------------------------------------------------------------
// DefaultProcessSpawner — real implementation using tokio::process
// ---------------------------------------------------------------------------

pub struct DefaultProcessSpawner;

struct TokioProcessHandle {
    child: Mutex<Option<tokio::process::Child>>,
}

impl ProcessHandle for TokioProcessHandle {
    fn kill(&self) {
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.start_kill();
            }
            *guard = None;
        }
    }

    fn is_alive(&self) -> bool {
        if let Ok(mut guard) = self.child.try_lock()
            && let Some(ref mut child) = *guard
        {
            return child.try_wait().ok().flatten().is_none();
        }
        false
    }
}

#[async_trait::async_trait]
impl ProcessSpawner for DefaultProcessSpawner {
    async fn spawn_officecli(
        &self,
        file_path: &str,
        port: u16,
        _doc_type: DocType,
    ) -> Result<Box<dyn ProcessHandle>, OfficeError> {
        let mut builder = CmdBuilder::new("officecli");
        builder
            .arg("watch")
            .arg(file_path)
            .arg("--port")
            .arg(port.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = builder.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OfficeError::OfficecliNotFound
            } else {
                OfficeError::StartFailed(e.to_string())
            }
        })?;

        Ok(Box::new(TokioProcessHandle {
            child: Mutex::new(Some(child)),
        }))
    }

    async fn install_officecli(&self) -> Result<(), OfficeError> {
        let mut builder = CmdBuilder::clean_cli("npm");
        builder.args(["install", "-g", "officecli"]);
        let output = builder
            .output()
            .await
            .map_err(|e| OfficeError::InstallFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OfficeError::InstallFailed(stderr.into_owned()));
        }
        Ok(())
    }

    async fn is_officecli_installed(&self) -> bool {
        let mut builder = CmdBuilder::clean_cli("officecli");
        builder.arg("--version");
        builder.output().await.is_ok_and(|o| o.status.success())
    }

    async fn check_update(&self, _doc_type: DocType) -> Result<(), OfficeError> {
        let mut builder = CmdBuilder::clean_cli("npm");
        builder.args(["outdated", "-g", "officecli"]);
        let output = builder
            .output()
            .await
            .map_err(|e| OfficeError::StartFailed(e.to_string()))?;

        if !output.status.success() {
            tracing::info!("officecli update available, installing...");
            let mut install_builder = CmdBuilder::clean_cli("npm");
            install_builder.args(["install", "-g", "officecli@latest"]);
            let install = install_builder
                .output()
                .await
                .map_err(|e| OfficeError::InstallFailed(e.to_string()))?;

            if !install.status.success() {
                let stderr = String::from_utf8_lossy(&install.stderr);
                tracing::warn!("officecli update failed: {stderr}");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_path(file_path: &str) -> Result<String, OfficeError> {
    let path = std::path::Path::new(file_path);
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Ok(resolved.to_string_lossy().into_owned())
}

fn session_key(resolved_path: &str, doc_type: DocType) -> String {
    format!("{doc_type}:{resolved_path}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    struct MockProcessHandle {
        alive: AtomicBool,
        killed: AtomicBool,
    }

    impl MockProcessHandle {
        fn new() -> Self {
            Self {
                alive: AtomicBool::new(true),
                killed: AtomicBool::new(false),
            }
        }
    }

    impl ProcessHandle for MockProcessHandle {
        fn kill(&self) {
            self.alive.store(false, Ordering::SeqCst);
            self.killed.store(true, Ordering::SeqCst);
        }

        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

    struct MockSpawner {
        installed: AtomicBool,
        spawn_count: AtomicU32,
        install_count: AtomicU32,
        update_count: AtomicU32,
        fail_spawn: AtomicBool,
        start_listener: AtomicBool,
    }

    impl MockSpawner {
        fn new() -> Self {
            Self {
                installed: AtomicBool::new(true),
                spawn_count: AtomicU32::new(0),
                install_count: AtomicU32::new(0),
                update_count: AtomicU32::new(0),
                fail_spawn: AtomicBool::new(false),
                start_listener: AtomicBool::new(true),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProcessSpawner for MockSpawner {
        async fn spawn_officecli(
            &self,
            _file_path: &str,
            port: u16,
            _doc_type: DocType,
        ) -> Result<Box<dyn ProcessHandle>, OfficeError> {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);

            if self.fail_spawn.load(Ordering::SeqCst) {
                return Err(OfficeError::StartFailed("mock spawn failure".into()));
            }

            if !self.installed.load(Ordering::SeqCst) {
                return Err(OfficeError::OfficecliNotFound);
            }

            if self.start_listener.load(Ordering::SeqCst) {
                let listener = std::net::TcpListener::bind(format!("127.0.0.1:{port}"))
                    .map_err(|e| OfficeError::StartFailed(e.to_string()))?;
                std::mem::forget(listener);
            }

            Ok(Box::new(MockProcessHandle::new()))
        }

        async fn install_officecli(&self) -> Result<(), OfficeError> {
            self.install_count.fetch_add(1, Ordering::SeqCst);
            self.installed.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn is_officecli_installed(&self) -> bool {
            self.installed.load(Ordering::SeqCst)
        }

        async fn check_update(&self, _doc_type: DocType) -> Result<(), OfficeError> {
            self.update_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct RecordingBroadcaster {
        events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }

    impl RecordingBroadcaster {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventBroadcaster for RecordingBroadcaster {
        fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn make_manager(spawner: Arc<MockSpawner>, broadcaster: Arc<RecordingBroadcaster>) -> OfficecliWatchManager {
        OfficecliWatchManager::new(spawner, broadcaster)
    }

    #[test]
    fn session_key_format() {
        let key = session_key("/path/to/doc.docx", DocType::Word);
        assert_eq!(key, "word:/path/to/doc.docx");
    }

    #[test]
    fn session_key_different_doc_types() {
        let k1 = session_key("/a.docx", DocType::Word);
        let k2 = session_key("/a.docx", DocType::Excel);
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn start_creates_session() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let port = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();
        assert!(port > 0);
        assert_eq!(mgr.active_session_count(), 1);
        assert_eq!(spawner.spawn_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn start_reuses_existing_session() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let p1 = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();
        let p2 = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();

        assert_eq!(p1, p2);
        assert_eq!(spawner.spawn_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn start_different_doc_types_independent() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let p1 = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();
        let p2 = mgr.start(file.to_str().unwrap(), DocType::Excel).await.unwrap();

        assert_ne!(p1, p2);
        assert_eq!(mgr.active_session_count(), 2);
        assert_eq!(spawner.spawn_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stop_removes_session() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();
        let path = file.to_str().unwrap();

        mgr.start(path, DocType::Word).await.unwrap();
        assert_eq!(mgr.active_session_count(), 1);

        mgr.stop(path, DocType::Word).await;
        assert_eq!(mgr.active_session_count(), 0);
    }

    #[tokio::test]
    async fn stop_all_clears_everything() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("a.docx");
        let f2 = dir.path().join("b.xlsx");
        std::fs::write(&f1, b"a").unwrap();
        std::fs::write(&f2, b"b").unwrap();

        mgr.start(f1.to_str().unwrap(), DocType::Word).await.unwrap();
        mgr.start(f2.to_str().unwrap(), DocType::Excel).await.unwrap();
        assert_eq!(mgr.active_session_count(), 2);

        mgr.stop_all();
        assert_eq!(mgr.active_session_count(), 0);
    }

    #[tokio::test]
    async fn is_active_port_returns_true_for_active() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let port = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();
        assert!(mgr.is_active_port(port, DocType::Word));
        assert!(!mgr.is_active_port(port, DocType::Ppt));
        assert!(!mgr.is_active_port(12345, DocType::Word));
    }

    #[tokio::test]
    async fn is_active_watch_port_accepts_word_and_excel() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let word_file = dir.path().join("test.docx");
        let excel_file = dir.path().join("test.xlsx");
        let ppt_file = dir.path().join("test.pptx");
        std::fs::write(&word_file, b"w").unwrap();
        std::fs::write(&excel_file, b"e").unwrap();
        std::fs::write(&ppt_file, b"p").unwrap();

        let word_port = mgr.start(word_file.to_str().unwrap(), DocType::Word).await.unwrap();
        let excel_port = mgr.start(excel_file.to_str().unwrap(), DocType::Excel).await.unwrap();
        let ppt_port = mgr.start(ppt_file.to_str().unwrap(), DocType::Ppt).await.unwrap();

        assert!(mgr.is_active_watch_port(word_port));
        assert!(mgr.is_active_watch_port(excel_port));
        assert!(!mgr.is_active_watch_port(ppt_port));
        assert!(!mgr.is_active_watch_port(12345));
    }

    #[tokio::test]
    async fn auto_install_when_not_found() {
        let spawner = Arc::new(MockSpawner::new());
        spawner.installed.store(false, Ordering::SeqCst);
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let port = mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();
        assert!(port > 0);
        assert_eq!(spawner.install_count.load(Ordering::SeqCst), 1);
        // First spawn fails (not installed), then install, then second spawn succeeds
        assert_eq!(spawner.spawn_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn broadcasts_starting_and_ready() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();

        let events = broadcaster.events();
        assert!(events.len() >= 2);
        assert_eq!(events[0].name, "word-preview.status");
        assert_eq!(events[0].data["state"], "starting");
        assert_eq!(events[1].name, "word-preview.status");
        assert_eq!(events[1].data["state"], "ready");
    }

    #[tokio::test]
    async fn broadcasts_installing_on_auto_install() {
        let spawner = Arc::new(MockSpawner::new());
        spawner.installed.store(false, Ordering::SeqCst);
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();

        let events = broadcaster.events();
        let states: Vec<&str> = events.iter().filter_map(|e| e.data["state"].as_str()).collect();
        assert!(states.contains(&"starting"));
        assert!(states.contains(&"installing"));
        assert!(states.contains(&"ready"));
    }

    #[tokio::test]
    async fn broadcasts_error_on_failure() {
        let spawner = Arc::new(MockSpawner::new());
        spawner.fail_spawn.store(true, Ordering::SeqCst);
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let result = mgr.start(file.to_str().unwrap(), DocType::Word).await;
        assert!(result.is_err());

        let events = broadcaster.events();
        let last = events.last().unwrap();
        assert_eq!(last.data["state"], "error");
    }

    #[tokio::test(start_paused = true)]
    async fn port_timeout_on_no_listener() {
        let spawner = Arc::new(MockSpawner::new());
        spawner.start_listener.store(false, Ordering::SeqCst);
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let resolved = resolve_path(file.to_str().unwrap()).unwrap();
        let port = allocate_port().unwrap();
        let result = mgr.poll_port_ready(port, &resolved).await;
        assert!(matches!(result, Err(OfficeError::PortTimeout(_))));
    }

    #[tokio::test]
    async fn ppt_triggers_version_check() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.pptx");
        std::fs::write(&file, b"test").unwrap();

        mgr.start(file.to_str().unwrap(), DocType::Ppt).await.unwrap();

        // Give the spawned task a moment
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(spawner.update_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn word_does_not_trigger_version_check() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(Arc::clone(&spawner), Arc::clone(&broadcaster));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        mgr.start(file.to_str().unwrap(), DocType::Word).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(spawner.update_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stop_nonexistent_is_no_op() {
        let spawner = Arc::new(MockSpawner::new());
        let broadcaster = Arc::new(RecordingBroadcaster::new());
        let mgr = make_manager(spawner, broadcaster);

        mgr.stop("/nonexistent/file.docx", DocType::Word).await;
        assert_eq!(mgr.active_session_count(), 0);
    }

    #[test]
    fn resolve_path_normalizes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.docx");
        std::fs::write(&file, b"test").unwrap();

        let resolved = resolve_path(file.to_str().unwrap()).unwrap();
        assert!(!resolved.is_empty());
    }

    #[test]
    fn resolve_path_nonexistent_returns_original() {
        let result = resolve_path("/nonexistent/path/test.docx").unwrap();
        assert_eq!(result, "/nonexistent/path/test.docx");
    }
}
