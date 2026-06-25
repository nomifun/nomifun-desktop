use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::warn;

use nomifun_api_types::WebSocketMessage;
use nomifun_common::AppError;
use nomifun_realtime::EventBroadcaster;

use crate::types::{FileWatchEvent, OfficeFileAddedEvent};

/// Debounce duration for file watch events.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(200);

/// Office file extensions to match (lowercase).
const OFFICE_EXTENSIONS: &[&str] = &["pptx", "docx", "xlsx"];

// ---------------------------------------------------------------------------
// Pure helpers (testable without I/O)
// ---------------------------------------------------------------------------

/// Returns `true` if the file path has an Office document extension.
fn is_office_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
        let lower = ext.to_ascii_lowercase();
        OFFICE_EXTENSIONS.contains(&lower.as_str())
    })
}

/// Maps a `notify::EventKind` to a human-readable event type string.
/// Returns `None` for events that should be silently skipped (e.g. access).
fn event_kind_to_str(kind: &EventKind) -> Option<&'static str> {
    match kind {
        EventKind::Modify(_) => Some("change"),
        EventKind::Create(_) => Some("create"),
        EventKind::Remove(_) => Some("remove"),
        EventKind::Any | EventKind::Other => Some("change"),
        EventKind::Access(_) => None,
    }
}

/// Returns `true` if enough time has elapsed since the last event for `key`.
/// Updates the timestamp when returning `true`.
fn should_emit(debounce: &DashMap<String, Instant>, key: &str) -> bool {
    let now = Instant::now();
    if let Some(last) = debounce.get(key)
        && now.duration_since(*last) < DEBOUNCE_DURATION
    {
        return false;
    }
    debounce.insert(key.to_owned(), now);
    true
}

// ---------------------------------------------------------------------------
// FileWatchService
// ---------------------------------------------------------------------------

/// File-system watcher implementing [`crate::traits::IFileWatchService`].
///
/// Internally uses the `notify` crate for cross-platform file-system events.
///
/// - **Single-file watches** share one [`RecommendedWatcher`] instance; each
///   path is registered via `watch()` with [`RecursiveMode::NonRecursive`].
/// - **Workspace Office watches** each get their own watcher running in
///   [`RecursiveMode::Recursive`], filtering for `.pptx`/`.docx`/`.xlsx`
///   creation events.
pub struct FileWatchService {
    broadcaster: Arc<dyn EventBroadcaster>,
    /// Shared watcher for all single-file watches.
    file_watcher: Mutex<RecommendedWatcher>,
    /// Set of canonical paths being watched (shared with the event handler).
    watched_files: Arc<DashMap<String, ()>>,
    /// Per-workspace Office watchers, keyed by canonical workspace path.
    office_watchers: Mutex<HashMap<String, RecommendedWatcher>>,
    /// Debounce timestamps shared with watcher callbacks.
    debounce: Arc<DashMap<String, Instant>>,
}

impl FileWatchService {
    /// Create a new watch service backed by the platform's recommended watcher.
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Result<Self, AppError> {
        let watched_files: Arc<DashMap<String, ()>> = Arc::new(DashMap::new());
        let debounce: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());

        let bc = broadcaster.clone();
        let wf = watched_files.clone();
        let db = debounce.clone();

        let file_watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "file watcher error");
                    return;
                }
            };

            let event_type = match event_kind_to_str(&event.kind) {
                Some(t) => t,
                None => return,
            };

            for path in &event.paths {
                let path_str = path.to_string_lossy().into_owned();
                if !wf.contains_key(&path_str) {
                    continue;
                }
                if !should_emit(&db, &path_str) {
                    continue;
                }
                let payload = FileWatchEvent {
                    file_path: path_str,
                    event_type: event_type.to_owned(),
                };
                let json = serde_json::to_value(&payload).unwrap_or_default();
                bc.broadcast(WebSocketMessage::new("fileWatch.fileChanged", json));
            }
        })
        .map_err(|e| AppError::Internal(format!("failed to create file watcher: {e}")))?;

        Ok(Self {
            broadcaster,
            file_watcher: Mutex::new(file_watcher),
            watched_files,
            office_watchers: Mutex::new(HashMap::new()),
            debounce,
        })
    }
}

#[async_trait::async_trait]
impl crate::traits::IFileWatchService for FileWatchService {
    async fn start_watch(&self, file_path: &str) -> Result<(), AppError> {
        let canonical = std::fs::canonicalize(file_path)
            .map_err(|e| AppError::NotFound(format!("cannot resolve path {file_path}: {e}")))?;
        let key = canonical.to_string_lossy().into_owned();

        // Idempotent: already watching → no-op.
        if self.watched_files.contains_key(&key) {
            return Ok(());
        }

        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        watcher
            .watch(&canonical, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Internal(format!("failed to watch {file_path}: {e}")))?;
        self.watched_files.insert(key, ());
        Ok(())
    }

    async fn stop_watch(&self, file_path: &str) -> Result<(), AppError> {
        let canonical = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.into());
        let key = canonical.to_string_lossy().into_owned();

        if self.watched_files.remove(&key).is_none() {
            return Ok(());
        }

        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        // Ignore unwatch errors — the file may have been deleted.
        let _ = watcher.unwatch(&canonical);
        self.debounce.remove(&key);
        Ok(())
    }

    async fn stop_all_watches(&self) -> Result<(), AppError> {
        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;

        for entry in self.watched_files.iter() {
            let path = std::path::PathBuf::from(entry.key().as_str());
            let _ = watcher.unwatch(&path);
        }
        self.watched_files.clear();
        // Clean file-watch debounce entries only (keep office ones).
        self.debounce.retain(|k, _| k.starts_with("office:"));
        Ok(())
    }

    async fn start_office_watch(&self, workspace: &str) -> Result<(), AppError> {
        let canonical = std::fs::canonicalize(workspace)
            .map_err(|e| AppError::NotFound(format!("cannot resolve workspace {workspace}: {e}")))?;
        let key = canonical.to_string_lossy().into_owned();

        {
            let watchers = self
                .office_watchers
                .lock()
                .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
            if watchers.contains_key(&key) {
                return Ok(());
            }
        }

        let bc = self.broadcaster.clone();
        let db = self.debounce.clone();
        let ws = key.clone();

        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "office watcher error");
                    return;
                }
            };

            if !matches!(event.kind, EventKind::Create(_)) {
                return;
            }

            for path in &event.paths {
                if !is_office_file(path) {
                    continue;
                }
                let path_str = path.to_string_lossy().into_owned();
                let debounce_key = format!("office:{path_str}");
                if !should_emit(&db, &debounce_key) {
                    continue;
                }
                let payload = OfficeFileAddedEvent {
                    file_path: path_str,
                    workspace: ws.clone(),
                };
                let json = serde_json::to_value(&payload).unwrap_or_default();
                bc.broadcast(WebSocketMessage::new("workspaceOfficeWatch.fileAdded", json));
            }
        })
        .map_err(|e| AppError::Internal(format!("failed to create office watcher: {e}")))?;

        watcher
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(|e| AppError::Internal(format!("failed to watch workspace {workspace}: {e}")))?;

        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        watchers.insert(key, watcher);
        Ok(())
    }

    async fn stop_office_watch(&self, workspace: &str) -> Result<(), AppError> {
        let canonical = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.into());
        let key = canonical.to_string_lossy().into_owned();

        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        // Dropping the watcher stops watching.
        watchers.remove(&key);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
    use std::path::PathBuf;

    // -- is_office_file --

    #[test]
    fn office_file_pptx() {
        assert!(is_office_file(Path::new("/ws/slides.pptx")));
    }

    #[test]
    fn office_file_docx() {
        assert!(is_office_file(Path::new("/ws/report.docx")));
    }

    #[test]
    fn office_file_xlsx() {
        assert!(is_office_file(Path::new("/ws/data.xlsx")));
    }

    #[test]
    fn office_file_case_insensitive() {
        assert!(is_office_file(Path::new("/ws/FILE.PPTX")));
        assert!(is_office_file(Path::new("/ws/Doc.Docx")));
    }

    #[test]
    fn non_office_file_txt() {
        assert!(!is_office_file(Path::new("/ws/readme.txt")));
    }

    #[test]
    fn non_office_file_pdf() {
        assert!(!is_office_file(Path::new("/ws/paper.pdf")));
    }

    #[test]
    fn no_extension() {
        assert!(!is_office_file(Path::new("/ws/Makefile")));
    }

    // -- event_kind_to_str --

    #[test]
    fn modify_event_maps_to_change() {
        assert_eq!(
            event_kind_to_str(&EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content))),
            Some("change")
        );
    }

    #[test]
    fn create_event_maps_to_create() {
        assert_eq!(event_kind_to_str(&EventKind::Create(CreateKind::File)), Some("create"));
    }

    #[test]
    fn remove_event_maps_to_remove() {
        assert_eq!(event_kind_to_str(&EventKind::Remove(RemoveKind::File)), Some("remove"));
    }

    #[test]
    fn any_event_maps_to_change() {
        assert_eq!(event_kind_to_str(&EventKind::Any), Some("change"));
    }

    #[test]
    fn other_event_maps_to_change() {
        assert_eq!(event_kind_to_str(&EventKind::Other), Some("change"));
    }

    #[test]
    fn access_event_is_skipped() {
        assert_eq!(event_kind_to_str(&EventKind::Access(AccessKind::Read)), None);
    }

    // -- should_emit (debounce) --

    #[test]
    fn first_emit_returns_true() {
        let db = DashMap::new();
        assert!(should_emit(&db, "/tmp/a.txt"));
    }

    #[test]
    fn immediate_second_emit_returns_false() {
        let db = DashMap::new();
        assert!(should_emit(&db, "/tmp/a.txt"));
        assert!(!should_emit(&db, "/tmp/a.txt"));
    }

    #[test]
    fn different_keys_are_independent() {
        let db = DashMap::new();
        assert!(should_emit(&db, "/tmp/a.txt"));
        assert!(should_emit(&db, "/tmp/b.txt"));
    }

    #[test]
    fn emit_after_debounce_duration() {
        let db = DashMap::new();
        assert!(should_emit(&db, "/tmp/a.txt"));

        // Simulate time passing by manually backdating the entry.
        db.insert(
            "/tmp/a.txt".to_owned(),
            Instant::now() - DEBOUNCE_DURATION - Duration::from_millis(1),
        );
        assert!(should_emit(&db, "/tmp/a.txt"));
    }

    // -- is_office_file edge cases --

    #[test]
    fn dotfile_with_office_ext() {
        assert!(is_office_file(Path::new("/ws/.hidden.docx")));
    }

    #[test]
    fn nested_path_office_file() {
        assert!(is_office_file(Path::new("/ws/deep/nested/dir/report.xlsx")));
    }

    #[test]
    fn empty_path() {
        assert!(!is_office_file(&PathBuf::new()));
    }
}
