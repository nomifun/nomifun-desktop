use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, warn};

use crate::constants::STATE_PERSIST_DEBOUNCE_MS;
use crate::error::ExtensionError;
use crate::types::ExtensionState;

// ---------------------------------------------------------------------------
// ExtensionStateStore
// ---------------------------------------------------------------------------

/// Manages loading and saving extension states to a JSON file with debounced
/// writes.
///
/// State is persisted to `extension-states.json`. Writes are debounced by
/// [`STATE_PERSIST_DEBOUNCE_MS`] to avoid excessive disk I/O when multiple
/// state changes happen in quick succession.
#[derive(Clone)]
pub struct ExtensionStateStore {
    inner: Arc<Inner>,
}

struct Inner {
    /// Path to the state JSON file.
    file_path: PathBuf,
    /// In-memory state map protected by a mutex.
    states: Mutex<HashMap<String, ExtensionState>>,
    /// Notifier used to trigger a debounced write.
    write_notify: Notify,
    /// Whether the background writer task has been spawned.
    writer_spawned: Mutex<bool>,
}

const EXTENSION_STATES_FILE_ENV: &str = "NOMIFUN_EXTENSION_STATES_FILE";
const DEFAULT_STATES_FILE: &str = "extension-states.json";

#[derive(Debug, Deserialize, Serialize)]
struct PersistedStates {
    version: u32,
    #[serde(default)]
    extensions: BTreeMap<String, PersistedExtensionState>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PersistedExtensionState {
    enabled: bool,
    #[serde(default)]
    installed: Option<bool>,
    #[serde(default, alias = "lastVersion", rename = "lastVersion")]
    last_version: Option<String>,
}

impl ExtensionStateStore {
    /// Create a new store backed by the given file path.
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                file_path,
                states: Mutex::new(HashMap::new()),
                write_notify: Notify::new(),
                writer_spawned: Mutex::new(false),
            }),
        }
    }

    /// Return the file path backing this store.
    pub fn file_path(&self) -> &Path {
        &self.inner.file_path
    }

    // -----------------------------------------------------------------------
    // Load
    // -----------------------------------------------------------------------

    /// Load persisted states from disk into memory.
    ///
    /// If the file does not exist, an empty map is used (all extensions will
    /// default to enabled). Parse errors are propagated as `ExtensionError`.
    pub async fn load(&self) -> Result<HashMap<String, ExtensionState>, ExtensionError> {
        let states = load_states_from_file(&self.inner.file_path)?;
        let mut guard = self.inner.states.lock().await;
        *guard = states.clone();
        Ok(states)
    }

    // -----------------------------------------------------------------------
    // Read helpers
    // -----------------------------------------------------------------------

    /// Get the persisted state for a single extension (or `None` if unknown).
    pub async fn get(&self, name: &str) -> Option<ExtensionState> {
        let guard = self.inner.states.lock().await;
        guard.get(name).cloned()
    }

    /// Snapshot of all current states.
    pub async fn get_all(&self) -> HashMap<String, ExtensionState> {
        let guard = self.inner.states.lock().await;
        guard.clone()
    }

    // -----------------------------------------------------------------------
    // Write (debounced)
    // -----------------------------------------------------------------------

    /// Update (or insert) the state for a single extension and schedule a
    /// debounced write to disk.
    pub async fn set(&self, state: ExtensionState) {
        {
            let mut guard = self.inner.states.lock().await;
            guard.insert(state.name.clone(), state);
        }
        self.schedule_write().await;
    }

    /// Replace the entire state map and schedule a debounced write.
    pub async fn set_all(&self, states: HashMap<String, ExtensionState>) {
        {
            let mut guard = self.inner.states.lock().await;
            *guard = states;
        }
        self.schedule_write().await;
    }

    /// Remove the persisted state for an extension and schedule a write.
    pub async fn remove(&self, name: &str) {
        {
            let mut guard = self.inner.states.lock().await;
            guard.remove(name);
        }
        self.schedule_write().await;
    }

    // -----------------------------------------------------------------------
    // Synchronous write (for shutdown or testing)
    // -----------------------------------------------------------------------

    /// Immediately write the current in-memory states to disk (no debounce).
    pub async fn flush(&self) -> Result<(), ExtensionError> {
        let snapshot = {
            let guard = self.inner.states.lock().await;
            guard.clone()
        };
        save_states_to_file(&self.inner.file_path, &snapshot)
    }

    // -----------------------------------------------------------------------
    // Debounce internals
    // -----------------------------------------------------------------------

    /// Notify the background writer that a write is pending. Spawns the
    /// background task on first call.
    async fn schedule_write(&self) {
        self.ensure_writer_spawned().await;
        self.inner.write_notify.notify_one();
    }

    /// Spawn the background debounce writer if not already running.
    async fn ensure_writer_spawned(&self) {
        let mut spawned = self.inner.writer_spawned.lock().await;
        if *spawned {
            return;
        }
        *spawned = true;

        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            loop {
                inner.write_notify.notified().await;

                // Debounce: wait for the configured duration, collapsing
                // additional notifications.
                tokio::time::sleep(std::time::Duration::from_millis(STATE_PERSIST_DEBOUNCE_MS)).await;

                let snapshot = {
                    let guard = inner.states.lock().await;
                    guard.clone()
                };

                if let Err(e) = save_states_to_file(&inner.file_path, &snapshot) {
                    error!(error = %e, "failed to persist extension states");
                } else {
                    debug!(path = %inner.file_path.display(), "extension states persisted");
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// File I/O (pure functions, no async needed)
// ---------------------------------------------------------------------------

/// Load extension states from a JSON file.
///
/// Returns an empty map if the file does not exist.
pub fn load_states_from_file(path: &Path) -> Result<HashMap<String, ExtensionState>, ExtensionError> {
    match std::fs::read(path) {
        Ok(bytes) => parse_states_from_bytes(path, &bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(path = %path.display(), "no state file found — starting fresh");
            Ok(HashMap::new())
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read state file");
            Err(ExtensionError::Io(e))
        }
    }
}

fn parse_states_from_bytes(path: &Path, bytes: &[u8]) -> Result<HashMap<String, ExtensionState>, ExtensionError> {
    let persisted: PersistedStates = serde_json::from_slice(bytes)?;
    if persisted.version != 1 {
        return Err(ExtensionError::StatePersistence(format!(
            "unsupported extension state file version {} at {}",
            persisted.version,
            path.display()
        )));
    }

    Ok(persisted
        .extensions
        .into_iter()
        .map(|(name, state)| {
            let version = state.last_version.unwrap_or_default();
            let installed_at = if state.installed == Some(true) { Some(0) } else { None };

            (
                name.clone(),
                ExtensionState {
                    name,
                    version,
                    enabled: state.enabled,
                    installed_at,
                    last_activated_at: None,
                },
            )
        })
        .collect())
}

/// Write extension states to a JSON file atomically.
///
/// Creates parent directories if they do not exist.
pub fn save_states_to_file(path: &Path, states: &HashMap<String, ExtensionState>) -> Result<(), ExtensionError> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
    }

    // Collect into a stable map keyed by extension name to match the
    // historical Electron format consumed by existing users.
    let mut names: Vec<&String> = states.keys().collect();
    names.sort();

    let mut extensions = BTreeMap::new();
    for name in names {
        let state = &states[name];
        extensions.insert(
            name.clone(),
            PersistedExtensionState {
                enabled: state.enabled,
                installed: state.installed_at.map(|_| true),
                last_version: (!state.version.is_empty()).then(|| state.version.clone()),
            },
        );
    }

    let json = serde_json::to_string_pretty(&PersistedStates { version: 1, extensions })?;

    // Write to a temp file then rename for atomicity.
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

/// Resolve the extension state file path using the historical Nomi rules.
///
/// Priority:
/// 1. `NOMIFUN_EXTENSION_STATES_FILE`
/// 2. `<data_dir>/extension-states.json`
pub fn resolve_state_file_path(data_dir: &Path) -> PathBuf {
    resolve_state_file_path_inner(std::env::var_os(EXTENSION_STATES_FILE_ENV).as_ref(), data_dir)
}

fn resolve_state_file_path_inner(override_path: Option<&std::ffi::OsString>, data_dir: &Path) -> PathBuf {
    if let Some(override_path) = override_path {
        let trimmed = override_path.to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    data_dir.join(DEFAULT_STATES_FILE)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::now_ms;
    use tempfile::TempDir;

    fn make_state(name: &str, version: &str, enabled: bool) -> ExtensionState {
        ExtensionState {
            name: name.to_string(),
            version: version.to_string(),
            enabled,
            installed_at: Some(now_ms()),
            last_activated_at: None,
        }
    }

    // -- load_states_from_file / save_states_to_file -------------------------

    #[test]
    fn load_nonexistent_file_returns_empty() {
        let result = load_states_from_file(Path::new("/nonexistent/states.json")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");

        let mut states = HashMap::new();
        states.insert("ext-a".to_string(), make_state("ext-a", "1.0.0", true));
        states.insert("ext-b".to_string(), make_state("ext-b", "2.0.0", false));

        save_states_to_file(&path, &states).unwrap();
        let loaded = load_states_from_file(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["ext-a"].version, "1.0.0");
        assert!(loaded["ext-a"].enabled);
        assert_eq!(loaded["ext-b"].version, "2.0.0");
        assert!(!loaded["ext-b"].enabled);
    }

    #[test]
    fn save_creates_parent_directories() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("dir").join("states.json");

        let states = HashMap::new();
        save_states_to_file(&path, &states).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_produces_sorted_output() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");

        let mut states = HashMap::new();
        states.insert("z-ext".to_string(), make_state("z-ext", "1.0.0", true));
        states.insert("a-ext".to_string(), make_state("a-ext", "1.0.0", true));

        save_states_to_file(&path, &states).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: PersistedStates = serde_json::from_str(&raw).unwrap();
        let ordered: Vec<&str> = parsed.extensions.keys().map(|k| k.as_str()).collect();
        assert_eq!(ordered, vec!["a-ext", "z-ext"]);
    }

    #[test]
    fn load_invalid_json_returns_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        std::fs::write(&path, b"not valid json").unwrap();

        let result = load_states_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_object_format_returns_states() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        std::fs::write(
            &path,
            r#"{
              "version": 1,
              "extensions": {
                "ext-a": {
                  "enabled": true,
                  "installed": true,
                  "lastVersion": "1.2.3"
                },
                "ext-b": {
                  "enabled": false
                }
              }
            }"#,
        )
        .unwrap();

        let loaded = load_states_from_file(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(loaded["ext-a"].enabled);
        assert_eq!(loaded["ext-a"].version, "1.2.3");
        assert!(!loaded["ext-b"].enabled);
        assert_eq!(loaded["ext-b"].version, "");
        assert!(loaded["ext-a"].installed_at.is_some());
        assert!(loaded["ext-a"].last_activated_at.is_none());
    }

    #[test]
    fn save_preserves_object_format() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        std::fs::write(
            &path,
            r#"{
              "version": 1,
              "extensions": {
                "ext-a": {
                  "enabled": true,
                  "lastVersion": "1.2.3"
                }
              }
            }"#,
        )
        .unwrap();

        let loaded = load_states_from_file(&path).unwrap();
        save_states_to_file(&path, &loaded).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: PersistedStates = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.extensions.len(), 1);
        assert_eq!(parsed.extensions["ext-a"].last_version.as_deref(), Some("1.2.3"));
        assert!(parsed.extensions["ext-a"].enabled);
    }

    #[test]
    fn resolve_state_file_path_prefers_data_dir() {
        let dir = TempDir::new().unwrap();
        let path = resolve_state_file_path_inner(None, dir.path());
        assert_eq!(path, dir.path().join("extension-states.json"));
    }

    #[test]
    fn resolve_state_file_path_honors_env_override() {
        let dir = TempDir::new().unwrap();
        let override_path = dir.path().join("custom-states.json");
        let override_os = override_path.as_os_str().to_os_string();
        let path = resolve_state_file_path_inner(Some(&override_os), Path::new("/ignored"));
        assert_eq!(path, override_path);
    }

    #[test]
    fn save_atomic_write() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");

        let mut states = HashMap::new();
        states.insert("ext-a".to_string(), make_state("ext-a", "1.0.0", true));
        save_states_to_file(&path, &states).unwrap();

        // Temp file should be cleaned up.
        let tmp_path = path.with_extension("json.tmp");
        assert!(!tmp_path.exists());
    }

    // -- ExtensionStateStore (async) ------------------------------------------

    #[tokio::test]
    async fn store_load_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.json");
        let store = ExtensionStateStore::new(path);

        let states = store.load().await.unwrap();
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn store_set_and_get() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        let store = ExtensionStateStore::new(path);

        store.load().await.unwrap();
        store.set(make_state("ext-a", "1.0.0", true)).await;

        let state = store.get("ext-a").await;
        assert!(state.is_some());
        assert!(state.unwrap().enabled);
    }

    #[tokio::test]
    async fn store_set_all_replaces_everything() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        let store = ExtensionStateStore::new(path);

        store.load().await.unwrap();
        store.set(make_state("old", "1.0.0", true)).await;

        let mut new_states = HashMap::new();
        new_states.insert("new".to_string(), make_state("new", "2.0.0", false));
        store.set_all(new_states).await;

        assert!(store.get("old").await.is_none());
        assert!(store.get("new").await.is_some());
    }

    #[tokio::test]
    async fn store_remove() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        let store = ExtensionStateStore::new(path);

        store.load().await.unwrap();
        store.set(make_state("ext-a", "1.0.0", true)).await;
        assert!(store.get("ext-a").await.is_some());

        store.remove("ext-a").await;
        assert!(store.get("ext-a").await.is_none());
    }

    #[tokio::test]
    async fn store_flush_persists_to_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        let store = ExtensionStateStore::new(path.clone());

        store.load().await.unwrap();
        store.set(make_state("ext-a", "1.0.0", true)).await;
        store.flush().await.unwrap();

        // Verify file exists and contains the state.
        let loaded = load_states_from_file(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("ext-a"));
    }

    #[tokio::test]
    async fn store_load_restores_existing_states() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");

        // Pre-populate the file.
        let mut states = HashMap::new();
        states.insert("ext-a".to_string(), make_state("ext-a", "1.0.0", false));
        save_states_to_file(&path, &states).unwrap();

        // Load into a fresh store.
        let store = ExtensionStateStore::new(path);
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(!loaded["ext-a"].enabled);

        // Memory state matches.
        let state = store.get("ext-a").await.unwrap();
        assert!(!state.enabled);
    }

    #[tokio::test]
    async fn store_debounced_write() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("states.json");
        let store = ExtensionStateStore::new(path.clone());

        store.load().await.unwrap();

        // Multiple rapid writes should be collapsed.
        for i in 0..5 {
            store.set(make_state(&format!("ext-{i}"), "1.0.0", true)).await;
        }

        // Wait for debounce to settle.
        tokio::time::sleep(std::time::Duration::from_millis(STATE_PERSIST_DEBOUNCE_MS + 200)).await;

        let loaded = load_states_from_file(&path).unwrap();
        assert_eq!(loaded.len(), 5);
    }
}
