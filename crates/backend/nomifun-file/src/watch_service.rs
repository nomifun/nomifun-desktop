use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::warn;

use nomifun_api_types::WebSocketMessage;
use nomifun_common::{AppError, UserId};
use nomifun_realtime::UserEventSink;

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
    user_events: Arc<dyn UserEventSink>,
    /// Shared watcher for all single-file watches.
    file_watcher: Mutex<RecommendedWatcher>,
    /// Set of canonical paths being watched (shared with the event handler).
    watched_files: Arc<DashMap<String, HashSet<String>>>,
    /// Per-workspace Office watchers, keyed by canonical workspace path.
    office_watchers: Mutex<HashMap<String, OfficeWatchRegistration>>,
    /// Debounce timestamps shared with watcher callbacks.
    debounce: Arc<DashMap<String, Instant>>,
}

struct OfficeWatchRegistration {
    _watcher: RecommendedWatcher,
    owners: Arc<DashMap<String, ()>>,
}

impl FileWatchService {
    /// Create a new watch service backed by the platform's recommended watcher.
    pub fn new(user_events: Arc<dyn UserEventSink>) -> Result<Self, AppError> {
        let watched_files: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());
        let debounce: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());

        let events = user_events.clone();
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
                let Some(owners) = wf.get(&path_str) else { continue };
                let owner_ids: Vec<String> = owners.iter().cloned().collect();
                drop(owners);
                if !should_emit(&db, &path_str) {
                    continue;
                }
                let payload = FileWatchEvent {
                    file_path: path_str,
                    event_type: event_type.to_owned(),
                };
                let json = serde_json::to_value(&payload).unwrap_or_default();
                for owner_id in owner_ids {
                    events.send_to_user(
                        &owner_id,
                        WebSocketMessage::new("fileWatch.fileChanged", json.clone()),
                    );
                }
            }
        })
        .map_err(|e| AppError::Internal(format!("failed to create file watcher: {e}")))?;

        Ok(Self {
            user_events,
            file_watcher: Mutex::new(file_watcher),
            watched_files,
            office_watchers: Mutex::new(HashMap::new()),
            debounce,
        })
    }
}

#[async_trait::async_trait]
impl crate::traits::IFileWatchService for FileWatchService {
    async fn start_watch(&self, owner_id: &str, file_path: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let canonical = std::fs::canonicalize(file_path)
            .map_err(|e| AppError::NotFound(format!("cannot resolve path {file_path}: {e}")))?;
        let key = canonical.to_string_lossy().into_owned();

        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        // The watcher lock serializes check/register/insert, so concurrent
        // owners cannot install duplicate OS watches for the same path.
        if let Some(mut owners) = self.watched_files.get_mut(&key) {
            owners.insert(owner_id.to_owned());
            return Ok(());
        }
        self.watched_files
            .insert(key.clone(), HashSet::from([owner_id.to_owned()]));
        if let Err(error) = watcher.watch(&canonical, RecursiveMode::NonRecursive) {
            self.watched_files.remove(&key);
            return Err(AppError::Internal(format!("failed to watch {file_path}: {error}")));
        }
        Ok(())
    }

    async fn stop_watch(&self, owner_id: &str, file_path: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let canonical = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.into());
        let key = canonical.to_string_lossy().into_owned();

        let remove_os_watch = match self.watched_files.get_mut(&key) {
            Some(mut owners) => {
                owners.remove(owner_id);
                owners.is_empty()
            }
            None => false,
        };
        if !remove_os_watch {
            return Ok(());
        }
        self.watched_files.remove(&key);

        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        // Ignore unwatch errors — the file may have been deleted.
        let _ = watcher.unwatch(&canonical);
        self.debounce.remove(&key);
        Ok(())
    }

    async fn stop_all_watches(&self, owner_id: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let paths: Vec<String> = self
            .watched_files
            .iter()
            .filter(|entry| entry.value().contains(owner_id))
            .map(|entry| entry.key().clone())
            .collect();
        for path in paths {
            self.stop_watch(owner_id, &path).await?;
        }
        Ok(())
    }

    async fn start_office_watch(&self, owner_id: &str, workspace: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let canonical = std::fs::canonicalize(workspace)
            .map_err(|e| AppError::NotFound(format!("cannot resolve workspace {workspace}: {e}")))?;
        let key = canonical.to_string_lossy().into_owned();

        // Keep the registration lock through watcher construction and insert.
        // Otherwise two concurrent callers can replace each other's watcher
        // and owner set, leaving an orphan callback alive.
        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        if let Some(registration) = watchers.get(&key) {
            registration.owners.insert(owner_id.to_owned(), ());
            return Ok(());
        }

        let events = self.user_events.clone();
        let db = self.debounce.clone();
        let ws = key.clone();
        let owners = Arc::new(DashMap::new());
        owners.insert(owner_id.to_owned(), ());
        let callback_owners = owners.clone();

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
                let owner_ids: Vec<String> = callback_owners.iter().map(|entry| entry.key().clone()).collect();
                for owner_id in owner_ids {
                    events.send_to_user(
                        &owner_id,
                        WebSocketMessage::new("workspaceOfficeWatch.fileAdded", json.clone()),
                    );
                }
            }
        })
        .map_err(|e| AppError::Internal(format!("failed to create office watcher: {e}")))?;

        watcher
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(|e| AppError::Internal(format!("failed to watch workspace {workspace}: {e}")))?;

        watchers.insert(
            key,
            OfficeWatchRegistration {
                _watcher: watcher,
                owners,
            },
        );
        Ok(())
    }

    async fn stop_office_watch(&self, owner_id: &str, workspace: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let canonical = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.into());
        let key = canonical.to_string_lossy().into_owned();

        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        let remove_registration = watchers
            .get(&key)
            .map(|registration| {
                registration.owners.remove(owner_id);
                registration.owners.is_empty()
            })
            .unwrap_or(false);
        if remove_registration {
            // Dropping the last owner's registration stops the OS watcher.
            watchers.remove(&key);
        }
        Ok(())
    }
}

fn require_owner(owner_id: &str) -> Result<(), AppError> {
    UserId::parse(owner_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid file watch owner: {error}")))
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
