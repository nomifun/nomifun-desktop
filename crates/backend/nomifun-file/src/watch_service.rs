use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
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
    should_emit_at(debounce, key, Instant::now())
}

fn should_emit_at(debounce: &DashMap<String, Instant>, key: &str, now: Instant) -> bool {
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
    /// Cache owner shared with the file routes; callbacks must not keep it alive.
    inventory: Weak<crate::FileService>,
    /// Shared watcher for all single-file watches.
    file_watcher: Mutex<FileWatchState>,
    /// Set of canonical paths being watched (shared with the event handler).
    watched_files: Arc<DashMap<String, HashSet<String>>>,
    /// Per-workspace Office watchers, keyed by canonical workspace path.
    office_watchers: Mutex<OfficeWatchState>,
    /// Debounce timestamps for the shared single-file watcher.
    debounce: Arc<DashMap<String, Instant>>,
}

fn native_inventory_changed(result: &Result<notify::Event, notify::Error>) -> bool {
    result.as_ref().map_or(true, |event| {
        event.need_rescan() || event_kind_to_str(&event.kind).is_some()
    })
}

fn invalidate_file_inventory(
    inventory: &Weak<crate::FileService>, watched: &DashMap<String, HashSet<String>>,
    result: &Result<notify::Event, notify::Error>,
) {
    if !native_inventory_changed(result) { return; }
    let Some(files) = inventory.upgrade() else { return; };
    if let Ok(event) = result && !event.need_rescan() {
        for path in &event.paths { files.invalidate_caches_for_path(path); }
    } else {
        // Native loss does not identify the affected file. Revoke every cache
        // intersecting a registration owned by this shared watcher.
        let paths: Vec<_> = watched.iter().map(|entry| entry.key().clone()).collect();
        for path in paths { files.invalidate_caches_for_path(Path::new(&path)); }
    }
}

fn invalidate_office_inventory(
    inventory: &Weak<crate::FileService>, workspace: &Path,
    result: &Result<notify::Event, notify::Error>,
) {
    if native_inventory_changed(result) && let Some(files) = inventory.upgrade() {
        // Every change matters to the inventory, including non-Office files,
        // ignore rules, renames and stream errors. Filter UI events afterwards.
        files.invalidate_caches_for_path(workspace);
    }
}

struct OfficeWatchRegistration {
    watcher: RecommendedWatcher,
    owners: Arc<DashMap<String, ()>>,
    debounce: Arc<DashMap<String, Instant>>,
}

fn emit_office_event(event: &notify::Event, workspace: &str, owners: &DashMap<String, ()>,
    debounce: &DashMap<String, Instant>, events: &dyn UserEventSink, now: Instant) {
    if !matches!(event.kind, EventKind::Create(_)) { return; }
    for path in &event.paths {
        if !is_office_file(path) { continue; }
        let path_str = path.to_string_lossy().into_owned();
        if !should_emit_at(debounce, &format!("office:{path_str}"), now) { continue; }
        let payload = OfficeFileAddedEvent { file_path: path_str, workspace: workspace.to_owned() };
        let json = serde_json::to_value(&payload).unwrap_or_default();
        let owner_ids: Vec<String> = owners.iter().map(|entry| entry.key().clone()).collect();
        for owner_id in owner_ids {
            events.send_to_user(&owner_id, WebSocketMessage::new("workspaceOfficeWatch.fileAdded", json.clone()));
        }
    }
}

type WatchAliases = HashMap<(String, PathBuf), String>;

struct FileWatchState { watcher: RecommendedWatcher, aliases: WatchAliases }

#[derive(Default)]
struct OfficeWatchState { registrations: HashMap<String, OfficeWatchRegistration>, aliases: WatchAliases }

fn watch_alias(path: &str) -> Result<PathBuf, AppError> {
    std::path::absolute(path).map_err(|error| AppError::BadRequest(format!("invalid watch path: {error}")))
}

fn watch_key(aliases: &WatchAliases, owner: &str, path: &str, registered: impl Fn(&str) -> bool) -> Result<String, AppError> {
    // A recorded request wins over a new filesystem resolution. The entry may
    // have disappeared, or a link may now point at a different registered file.
    if let Some(key) = aliases.get(&(owner.to_owned(), watch_alias(path)?)) { return Ok(key.clone()); }
    if registered(path) { return Ok(path.to_owned()); }
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.into()).to_string_lossy().into_owned())
}

fn validate_watch_alias(aliases: &WatchAliases, owner: &str, alias: &Path, canonical: &str) -> Result<(), AppError> {
    if aliases.get(&(owner.to_owned(), alias.to_path_buf())).is_some_and(|prior| prior != canonical) {
        return Err(AppError::Conflict("watch path changed target; stop the original subscription before restarting".into()));
    }
    Ok(())
}

fn forget_watch_aliases(aliases: &mut WatchAliases, owner: &str, canonical: &str) {
    aliases.retain(|(registered_owner, _), target| registered_owner != owner || target != canonical);
}

fn confirm_unwatch(watcher: &mut RecommendedWatcher, path: &Path) -> Result<(), AppError> {
    match watcher.unwatch(path) {
        Ok(()) => Ok(()),
        Err(error) if matches!(error.kind, notify::ErrorKind::WatchNotFound) => Ok(()),
        Err(error) => Err(AppError::Internal(format!("file watch cleanup is unconfirmed: {error}"))),
    }
}

impl FileWatchService {
    fn invalidate_registered_inventory(&self, path: &Path) {
        if let Some(files) = self.inventory.upgrade() {
            files.invalidate_caches_for_path(path);
        }
    }

    /// Create a new watch service backed by the platform's recommended watcher.
    pub fn new(user_events: Arc<dyn UserEventSink>, inventory: Weak<crate::FileService>) -> Result<Self, AppError> {
        let watched_files: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());
        let debounce: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());

        let events = user_events.clone();
        let wf = watched_files.clone();
        let db = debounce.clone();
        let callback_inventory = inventory.clone();

        let file_watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            invalidate_file_inventory(&callback_inventory, &wf, &res);
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
            inventory,
            file_watcher: Mutex::new(FileWatchState { watcher: file_watcher, aliases: HashMap::new() }),
            watched_files,
            office_watchers: Mutex::new(OfficeWatchState::default()),
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
        let alias = watch_alias(file_path)?;

        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        validate_watch_alias(&watcher.aliases, owner_id, &alias, &key)?;
        // The watcher lock serializes check/register/insert, so concurrent
        // owners cannot install duplicate OS watches for the same path.
        if let Some(mut owners) = self.watched_files.get_mut(&key) {
            owners.insert(owner_id.to_owned());
            watcher.aliases.insert((owner_id.to_owned(), alias), key);
            self.invalidate_registered_inventory(&canonical);
            return Ok(());
        }
        self.watched_files
            .insert(key.clone(), HashSet::from([owner_id.to_owned()]));
        if let Err(error) = watcher.watcher.watch(&canonical, RecursiveMode::NonRecursive) {
            self.watched_files.remove(&key);
            return Err(AppError::Internal(format!("failed to watch {file_path}: {error}")));
        }
        watcher.aliases.insert((owner_id.to_owned(), alias), key);
        // The watcher cannot report changes made before this subscription.
        self.invalidate_registered_inventory(&canonical);
        Ok(())
    }

    async fn stop_watch(&self, owner_id: &str, file_path: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        // Serialize owner removal and OS unregistration with start_watch.
        let mut watcher = self
            .file_watcher
            .lock()
            .map_err(|e| AppError::Internal(format!("file watcher lock poisoned: {e}")))?;
        let key = watch_key(&watcher.aliases, owner_id, file_path,
            |key| self.watched_files.get(key).is_some_and(|owners| owners.contains(owner_id)))?;
        let count = self.watched_files.get(&key).filter(|owners| owners.contains(owner_id)).map(|owners| owners.len());
        match count {
            None => return Ok(()),
            Some(1) => {
                confirm_unwatch(&mut watcher.watcher, Path::new(&key))?;
                self.watched_files.remove(&key);
                self.debounce.remove(&key);
            }
            Some(_) => { if let Some(mut owners) = self.watched_files.get_mut(&key) { owners.remove(owner_id); } }
        }
        forget_watch_aliases(&mut watcher.aliases, owner_id, &key);
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
        let alias = watch_alias(workspace)?;

        // Keep the registration lock through watcher construction and insert.
        // Otherwise two concurrent callers can replace each other's watcher
        // and owner set, leaving an orphan callback alive.
        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        validate_watch_alias(&watchers.aliases, owner_id, &alias, &key)?;
        if let Some(registration) = watchers.registrations.get(&key) {
            registration.owners.insert(owner_id.to_owned(), ());
            watchers.aliases.insert((owner_id.to_owned(), alias), key);
            self.invalidate_registered_inventory(&canonical);
            return Ok(());
        }

        let events = self.user_events.clone();
        // Different workspace registrations may overlap, and a restarted
        // registration must never inherit suppression from its predecessor.
        let db = Arc::new(DashMap::new());
        let registration_debounce = db.clone();
        let ws = key.clone();
        let owners = Arc::new(DashMap::new());
        owners.insert(owner_id.to_owned(), ());
        let callback_owners = owners.clone();
        let inventory = self.inventory.clone();

        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            invalidate_office_inventory(&inventory, Path::new(&ws), &res);
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "office watcher error");
                    return;
                }
            };

            emit_office_event(&event, &ws, &callback_owners, &db, events.as_ref(), Instant::now());
        })
        .map_err(|e| AppError::Internal(format!("failed to create office watcher: {e}")))?;

        watcher
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(|e| AppError::Internal(format!("failed to watch workspace {workspace}: {e}")))?;

        watchers.registrations.insert(
            key.clone(),
            OfficeWatchRegistration {
                watcher,
                owners,
                debounce: registration_debounce,
            },
        );
        watchers.aliases.insert((owner_id.to_owned(), alias), key);
        self.invalidate_registered_inventory(&canonical);
        Ok(())
    }

    async fn stop_office_watch(&self, owner_id: &str, workspace: &str) -> Result<(), AppError> {
        require_owner(owner_id)?;
        let mut watchers = self
            .office_watchers
            .lock()
            .map_err(|e| AppError::Internal(format!("office watcher lock poisoned: {e}")))?;
        let key = watch_key(&watchers.aliases, owner_id, workspace,
            |key| watchers.registrations.get(key).is_some_and(|registration| registration.owners.contains_key(owner_id)))?;
        let Some(registration) = watchers.registrations.get_mut(&key).filter(|registration| registration.owners.contains_key(owner_id)) else { return Ok(()); };
        let last_owner = registration.owners.len() == 1;
        if last_owner { confirm_unwatch(&mut registration.watcher, Path::new(&key))?; }
        registration.owners.remove(owner_id);
        if last_owner {
            registration.debounce.clear();
            watchers.registrations.remove(&key);
        }
        forget_watch_aliases(&mut watchers.aliases, owner_id, &key);
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

    struct NoEvents;
    impl UserEventSink for NoEvents {
        fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }

    fn lifecycle_fixture() -> tempfile::TempDir {
        let mut builder = tempfile::Builder::new();
        builder.prefix("watch-lifecycle-");
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    struct InventoryDeliveryEvents {
        files: Mutex<std::sync::Weak<crate::FileService>>,
        root: PathBuf,
        runtime: tokio::runtime::Handle,
        office: bool,
        observations: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<String>, String>)>,
    }

    impl UserEventSink for InventoryDeliveryEvents {
        fn send_to_user(&self, owner: &str, event: WebSocketMessage<serde_json::Value>) {
            let expected = if self.office {
                event.name == "workspaceOfficeWatch.fileAdded"
            } else {
                event.name == "fileWatch.fileChanged" && event.data["event_type"] == "remove"
            };
            if !expected { return; }
            let files = self.files.lock().unwrap().upgrade().unwrap();
            let root = self.root.clone();
            let runtime = self.runtime.clone();
            // Observe the public API during delivery, before the callback can
            // perform any later invalidation. The native callback has no runtime.
            let observed = std::thread::spawn(move || runtime.block_on(async move {
                use crate::IFileService;
                files.list_workspace_files(root.to_str().unwrap()).await
                    .map(|files| {
                        let mut names = files.into_iter().map(|file| file.name).collect::<Vec<_>>();
                        names.sort();
                        names
                    }).map_err(|error| error.to_string())
            })).join().unwrap();
            let _ = self.observations.send((owner.to_owned(), observed));
        }
    }

    async fn inventory_after_native_delivery(office: bool) {
        use crate::{IFileService, IFileWatchService};
        let fixture = lifecycle_fixture();
        let root = fixture.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let old = root.join("old.txt");
        std::fs::write(&old, b"old").unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let events = Arc::new(InventoryDeliveryEvents {
            files: Mutex::new(std::sync::Weak::new()), root: root.clone(),
            runtime: tokio::runtime::Handle::current(), office, observations: sender,
        });
        let files = Arc::new(crate::FileService::new(events.clone(), vec![root.clone()]));
        *events.files.lock().unwrap() = Arc::downgrade(&files);
        assert_eq!(files.list_workspace_files(root.to_str().unwrap()).await.unwrap().len(), 1);
        let watches = FileWatchService::new(events, Arc::downgrade(&files)).unwrap();
        let owner = nomifun_common::generate_id();
        if office {
            watches.start_office_watch(&owner, root.to_str().unwrap()).await.unwrap();
            assert_eq!(files.list_workspace_files(root.to_str().unwrap()).await.unwrap().len(), 1);
            std::fs::write(root.join("new.docx"), b"new").unwrap();
        } else {
            watches.start_watch(&owner, old.to_str().unwrap()).await.unwrap();
            assert_eq!(files.list_workspace_files(root.to_str().unwrap()).await.unwrap().len(), 1);
            std::fs::remove_file(old).unwrap();
        }
        let observed = tokio::time::timeout(Duration::from_secs(3), receiver.recv()).await;
        let mut actual = std::fs::read_dir(&root).unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        actual.sort();
        drop(watches);
        if !matches!(&observed, Ok(Some((recipient, Ok(names)))) if recipient == &owner && names == &actual) {
            let observation = serde_json::json!({ "office": office, "observed": format!("{observed:?}"), "disk": actual });
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&observation).unwrap()).unwrap();
            panic!("native delivery exposed a stale inventory; retained fixture: {}", fixture.keep().display());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inventory_refreshes_before_native_office_delivery() {
        inventory_after_native_delivery(true).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inventory_refreshes_before_native_remove_delivery() {
        inventory_after_native_delivery(false).await;
    }

    #[tokio::test]
    async fn inventory_activation_reconciles_changes_before_subscription() {
        use crate::{IFileService, IFileWatchService};
        let fixture = lifecycle_fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let files = Arc::new(crate::FileService::new(Arc::new(NoEvents), vec![root.clone()]));
        let watches = FileWatchService::new(Arc::new(NoEvents), Arc::downgrade(&files)).unwrap();
        let owner = nomifun_common::generate_id();
        let mut observations = Vec::new();
        for office in [false, true] {
            let workspace = root.join(if office { "office" } else { "single" });
            std::fs::create_dir(&workspace).unwrap();
            let watched = workspace.join("watched.txt");
            std::fs::write(&watched, b"old").unwrap();
            for round in 1..=2 {
                files.list_workspace_files(workspace.to_str().unwrap()).await.unwrap();
                // No subscription is active while the cached inventory becomes stale.
                std::fs::write(workspace.join(format!("gap-{round}.docx")), b"gap").unwrap();
                if office {
                    watches.start_office_watch(&owner, workspace.to_str().unwrap()).await.unwrap();
                } else {
                    watches.start_watch(&owner, watched.to_str().unwrap()).await.unwrap();
                }
                let observed = files.list_workspace_files(workspace.to_str().unwrap()).await.unwrap();
                let actual = std::fs::read_dir(&workspace).unwrap().count();
                observations.push((office, round, observed.len(), actual));
                if office {
                    watches.stop_office_watch(&owner, workspace.to_str().unwrap()).await.unwrap();
                } else {
                    watches.stop_watch(&owner, watched.to_str().unwrap()).await.unwrap();
                }
            }
        }
        drop(watches);
        if observations.iter().any(|(_, _, observed, actual)| observed != actual) {
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&observations).unwrap()).unwrap();
            panic!("activation kept a pre-subscription snapshot; retained fixture: {}", fixture.keep().display());
        }
    }

    #[tokio::test]
    async fn inventory_gaps_and_filtered_office_changes_revoke_cached_snapshots() {
        use crate::IFileService;
        let fixture = lifecycle_fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let files = Arc::new(crate::FileService::new(Arc::new(NoEvents), vec![root.clone()]));
        let first = root.join("first");
        let second = root.join("second");
        let watched = DashMap::new();
        for workspace in [&first, &second] {
            std::fs::create_dir(workspace).unwrap();
            let file = workspace.join("watched.txt");
            std::fs::write(&file, b"old").unwrap();
            watched.insert(file.to_string_lossy().into_owned(), HashSet::from(["owner".to_owned()]));
        }
        let events = [
            (false, Err(notify::Error::generic("injected native gap"))),
            (false, Ok(notify::Event::new(EventKind::Access(AccessKind::Any)).set_flag(notify::event::Flag::Rescan))),
            (true, Err(notify::Error::generic("injected native gap"))),
            (true, Ok(notify::Event::new(EventKind::Access(AccessKind::Any)).set_flag(notify::event::Flag::Rescan))),
            (true, Ok(notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(first.join("plain.txt")))),
        ];
        let mut observations = Vec::new();
        for (index, (office, event)) in events.iter().enumerate() {
            for workspace in [&first, &second] {
                files.list_workspace_files(workspace.to_str().unwrap()).await.unwrap();
                std::fs::write(workspace.join(format!("{index}.txt")), b"new").unwrap();
            }
            if *office {
                // A recursive registration covers all names, even when Office
                // delivery would filter out this event's kind or extension.
                invalidate_office_inventory(&Arc::downgrade(&files), &root, event);
            } else {
                invalidate_file_inventory(&Arc::downgrade(&files), &watched, event);
            }
            for workspace in [&first, &second] {
                let observed = files.list_workspace_files(workspace.to_str().unwrap()).await.unwrap().len();
                let actual = std::fs::read_dir(workspace).unwrap().count();
                observations.push((index, observed, actual));
            }
        }
        if observations.iter().any(|(_, observed, actual)| observed != actual) {
            std::fs::write(root.join("observation.json"), serde_json::to_vec_pretty(&observations).unwrap()).unwrap();
            panic!("native gap or filtered change kept stale caches; retained fixture: {}", fixture.keep().display());
        }
    }

    #[test]
    fn inventory_callbacks_do_not_retain_the_file_service() {
        let files = Arc::new(crate::FileService::new(Arc::new(NoEvents), vec![]));
        let weak = Arc::downgrade(&files);
        let watches = FileWatchService::new(Arc::new(NoEvents), weak.clone()).unwrap();
        drop(files);
        assert!(weak.upgrade().is_none());
        invalidate_file_inventory(&weak, &watches.watched_files, &Err(notify::Error::generic("closed")));
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn deleted_file_watch_can_be_stopped_through_its_original_alias() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        std::fs::create_dir(fixture.path().join("parent")).unwrap();
        let file = fixture.path().join("watched.txt");
        let alias = fixture.path().join("parent/../watched.txt");
        std::fs::write(&file, b"initial").unwrap();
        let owner = nomifun_common::generate_id();
        let service = FileWatchService::new(Arc::new(NoEvents), Weak::new()).unwrap();
        service.start_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::remove_file(&file).unwrap();
        service.stop_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        if !service.watched_files.is_empty() {
            let registrations = service.watched_files.iter().map(|entry| entry.key().clone()).collect::<Vec<_>>();
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&registrations).unwrap()).unwrap();
            panic!("deleted file subscription survived stop; retained fixture: {}", fixture.keep().display());
        }
        std::fs::write(&file, b"new file").unwrap();
        service.start_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        service.stop_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        assert!(service.watched_files.is_empty());
    }

    #[tokio::test]
    async fn deleted_office_workspace_can_be_stopped_through_its_original_alias() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        std::fs::create_dir(fixture.path().join("parent")).unwrap();
        let workspace = fixture.path().join("workspace");
        let alias = fixture.path().join("parent/../workspace");
        std::fs::create_dir(&workspace).unwrap();
        let owner = nomifun_common::generate_id();
        let service = FileWatchService::new(Arc::new(NoEvents), Weak::new()).unwrap();
        service.start_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::remove_dir(&workspace).unwrap();
        service.stop_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        if !service.office_watchers.lock().unwrap().registrations.is_empty() {
            let registrations = service.office_watchers.lock().unwrap().registrations.keys().cloned().collect::<Vec<_>>();
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&registrations).unwrap()).unwrap();
            panic!("deleted workspace subscription survived stop; retained fixture: {}", fixture.keep().display());
        }
        std::fs::create_dir(&workspace).unwrap();
        service.start_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        service.stop_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        assert!(service.office_watchers.lock().unwrap().registrations.is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn retargeted_alias_stops_its_original_subscription_without_affecting_another() {
        use crate::IFileWatchService;
        for office in [false, true] {
            let fixture = lifecycle_fixture();
            let first = fixture.path().join("first");
            let second = fixture.path().join("second");
            let alias = fixture.path().join("alias");
            std::fs::create_dir(&first).unwrap();
            std::fs::create_dir(&second).unwrap();
            std::fs::write(first.join("watched.txt"), b"first").unwrap();
            std::fs::write(second.join("watched.txt"), b"second").unwrap();
            junction::create(&first, &alias).unwrap();
            let original = if office { alias.clone() } else { alias.join("watched.txt") };
            let other = if office { second.clone() } else { second.join("watched.txt") };
            let other_key = std::fs::canonicalize(&other).unwrap().to_string_lossy().into_owned();
            let owner = nomifun_common::generate_id();
            let service = FileWatchService::new(Arc::new(NoEvents), Weak::new()).unwrap();
            if office {
                service.start_office_watch(&owner, original.to_str().unwrap()).await.unwrap();
                service.start_office_watch(&owner, other.to_str().unwrap()).await.unwrap();
            } else {
                service.start_watch(&owner, original.to_str().unwrap()).await.unwrap();
                service.start_watch(&owner, other.to_str().unwrap()).await.unwrap();
            }
            junction::delete(&alias).unwrap();
            std::fs::remove_dir(&alias).unwrap();
            junction::create(&second, &alias).unwrap();
            if office {
                assert!(matches!(service.start_office_watch(&owner, original.to_str().unwrap()).await, Err(AppError::Conflict(_))));
                service.stop_office_watch(&owner, original.to_str().unwrap()).await.unwrap();
                assert_eq!(service.office_watchers.lock().unwrap().registrations.len(), 1);
                assert!(service.office_watchers.lock().unwrap().registrations.contains_key(&other_key));
                service.stop_office_watch(&owner, other.to_str().unwrap()).await.unwrap();
                let state = service.office_watchers.lock().unwrap();
                assert!(state.registrations.is_empty() && state.aliases.is_empty());
            } else {
                assert!(matches!(service.start_watch(&owner, original.to_str().unwrap()).await, Err(AppError::Conflict(_))));
                service.stop_watch(&owner, original.to_str().unwrap()).await.unwrap();
                assert_eq!(service.watched_files.len(), 1);
                assert!(service.watched_files.contains_key(&other_key));
                service.stop_all_watches(&owner).await.unwrap();
                assert!(service.watched_files.is_empty());
                assert!(service.file_watcher.lock().unwrap().aliases.is_empty());
            }
            junction::delete(&alias).unwrap();
        }
    }

    struct ChannelEvents(tokio::sync::mpsc::UnboundedSender<(String, nomifun_api_types::WebSocketMessage<serde_json::Value>)>);
    impl UserEventSink for ChannelEvents {
        fn send_to_user(&self, owner: &str, event: nomifun_api_types::WebSocketMessage<serde_json::Value>) {
            let _ = self.0.send((owner.to_owned(), event));
        }
    }

    #[tokio::test]
    async fn recreated_watches_deliver_real_events_to_the_new_owner() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        std::fs::create_dir(fixture.path().join("parent")).unwrap();
        let owner = nomifun_common::generate_id();
        let next_owner = nomifun_common::generate_id();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let service = FileWatchService::new(Arc::new(ChannelEvents(sender)), Weak::new()).unwrap();
        let file = fixture.path().join("file.txt");
        let alias = fixture.path().join("parent/../file.txt");
        std::fs::write(&file, b"old").unwrap();
        service.start_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::remove_file(&file).unwrap();
        service.stop_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::write(&file, b"new").unwrap();
        service.start_watch(&next_owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::write(&file, b"modified").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let (recipient, event) = receiver.recv().await.unwrap();
                if recipient == next_owner && event.name == "fileWatch.fileChanged" { break; }
            }
        }).await.expect("recreated file watch did not receive a native event");
        service.stop_all_watches(&next_owner).await.unwrap();
        let workspace = fixture.path().join("workspace");
        let alias = fixture.path().join("parent/../workspace");
        std::fs::create_dir(&workspace).unwrap();
        service.start_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::remove_dir(&workspace).unwrap();
        service.stop_office_watch(&owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::create_dir(&workspace).unwrap();
        service.start_office_watch(&next_owner, alias.to_str().unwrap()).await.unwrap();
        std::fs::write(workspace.join("new.docx"), b"new office fixture").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let (recipient, event) = receiver.recv().await.unwrap();
                if recipient == next_owner && event.name == "workspaceOfficeWatch.fileAdded" { break; }
            }
        }).await.expect("recreated workspace watch did not receive a native event");
        service.stop_office_watch(&next_owner, alias.to_str().unwrap()).await.unwrap();
        assert!(service.watched_files.is_empty());
        assert!(service.file_watcher.lock().unwrap().aliases.is_empty());
        let office = service.office_watchers.lock().unwrap();
        assert!(office.registrations.is_empty() && office.aliases.is_empty());
    }

    #[derive(Default)]
    struct OfficeDeliveryEvents(Mutex<Vec<(String, String)>>);
    impl UserEventSink for OfficeDeliveryEvents {
        fn send_to_user(&self, owner: &str, event: nomifun_api_types::WebSocketMessage<serde_json::Value>) {
            if event.name == "workspaceOfficeWatch.fileAdded" {
                self.0.lock().unwrap().push((owner.to_owned(), event.data["workspace"].as_str().unwrap().to_owned()));
            }
        }
    }

    fn inject_office_event(service: &FileWatchService, root: &str, file: &Path, now: Instant, events: &OfficeDeliveryEvents) {
        let registrations = service.office_watchers.lock().unwrap();
        let registration = registrations.registrations.get(root).unwrap();
        let event = notify::Event::new(EventKind::Create(CreateKind::File)).add_path(file.to_path_buf());
        emit_office_event(&event, root, &registration.owners, &registration.debounce, events, now);
    }

    #[tokio::test]
    async fn overlapping_office_roots_each_deliver_the_same_native_event() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let file = nested.join("shared.docx");
        std::fs::write(&file, b"fixture").unwrap();
        let parent_owner = nomifun_common::generate_id();
        let child_owner = nomifun_common::generate_id();
        let events = Arc::new(OfficeDeliveryEvents::default());
        let service = FileWatchService::new(events.clone(), Weak::new()).unwrap();
        service.start_office_watch(&parent_owner, root.to_str().unwrap()).await.unwrap();
        service.start_office_watch(&child_owner, nested.to_str().unwrap()).await.unwrap();
        let now = Instant::now();
        inject_office_event(&service, root.to_str().unwrap(), &file, now, &events);
        inject_office_event(&service, nested.to_str().unwrap(), &file, now, &events);
        let observed = events.0.lock().unwrap().clone();
        if observed != [(parent_owner.clone(), root.to_string_lossy().into_owned()), (child_owner.clone(), nested.to_string_lossy().into_owned())] {
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&observed).unwrap()).unwrap();
            panic!("one workspace suppressed another's event; retained fixture: {}", fixture.keep().display());
        }
    }

    #[tokio::test]
    async fn office_restarts_do_not_inherit_the_prior_subscription_debounce() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let file = root.join("recreated.docx");
        std::fs::write(&file, b"fixture").unwrap();
        let old_owner = nomifun_common::generate_id();
        let new_owner = nomifun_common::generate_id();
        let events = Arc::new(OfficeDeliveryEvents::default());
        let service = FileWatchService::new(events.clone(), Weak::new()).unwrap();
        service.start_office_watch(&old_owner, root.to_str().unwrap()).await.unwrap();
        let now = Instant::now();
        inject_office_event(&service, root.to_str().unwrap(), &file, now, &events);
        service.stop_office_watch(&old_owner, root.to_str().unwrap()).await.unwrap();
        service.start_office_watch(&new_owner, root.to_str().unwrap()).await.unwrap();
        inject_office_event(&service, root.to_str().unwrap(), &file, now, &events);
        let observed = events.0.lock().unwrap().clone();
        if observed.iter().map(|item| &item.0).collect::<Vec<_>>() != [&old_owner, &new_owner] {
            std::fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&observed).unwrap()).unwrap();
            panic!("new subscription inherited old suppression; retained fixture: {}", fixture.keep().display());
        }
    }

    #[tokio::test]
    async fn overlapping_office_subscriptions_receive_real_events_independently() {
        use crate::IFileWatchService;
        let fixture = lifecycle_fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let parent_owner = nomifun_common::generate_id();
        let child_owner = nomifun_common::generate_id();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let service = FileWatchService::new(Arc::new(ChannelEvents(sender)), Weak::new()).unwrap();
        service.start_office_watch(&parent_owner, root.to_str().unwrap()).await.unwrap();
        service.start_office_watch(&child_owner, nested.to_str().unwrap()).await.unwrap();
        std::fs::write(nested.join("shared.docx"), b"first fixture").unwrap();
        let observed = tokio::time::timeout(Duration::from_secs(3), async {
            let mut observed = HashSet::new();
            while observed.len() < 2 {
                let (owner, event) = receiver.recv().await.unwrap();
                if event.name == "workspaceOfficeWatch.fileAdded" && event.data["file_path"].as_str().is_some_and(|path| path.ends_with("shared.docx")) {
                    observed.insert((owner, event.data["workspace"].as_str().unwrap().to_owned()));
                }
            }
            observed
        }).await.expect("both native workspace subscriptions must receive the event");
        assert_eq!(observed, HashSet::from([(parent_owner.clone(), root.to_string_lossy().into_owned()),
            (child_owner.clone(), nested.to_string_lossy().into_owned())]));
        service.stop_office_watch(&parent_owner, root.to_str().unwrap()).await.unwrap();
        std::fs::write(nested.join("second.docx"), b"second fixture").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let (owner, event) = receiver.recv().await.unwrap();
                if event.name == "workspaceOfficeWatch.fileAdded" && event.data["file_path"].as_str().is_some_and(|path| path.ends_with("second.docx")) {
                    assert_eq!(owner, child_owner);
                    assert_eq!(event.data["workspace"], nested.to_string_lossy().as_ref());
                    break;
                }
            }
        }).await.expect("remaining native subscriber must keep receiving events");
        service.stop_office_watch(&child_owner, nested.to_str().unwrap()).await.unwrap();
        assert!(service.office_watchers.lock().unwrap().registrations.is_empty());
        assert!(service.debounce.is_empty(), "Office paths must not accumulate in the shared file-watcher debounce map");
    }

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
