//! Integration tests for state persistence (test-plan SP-1..SP-3).
//!
//! These tests exercise `ExtensionStateStore` and the underlying file I/O
//! as black-box functionality: saving states after enable/disable, restoring
//! states across "restarts", and default-enabled behaviour on first launch.

use std::collections::HashMap;

use nomifun_extension::{ExtensionState, ExtensionStateStore, load_states_from_file, save_states_to_file};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(name: &str, version: &str, enabled: bool) -> ExtensionState {
    ExtensionState {
        name: name.to_string(),
        version: version.to_string(),
        enabled,
        installed_at: Some(1_700_000_000_000),
        last_activated_at: None,
    }
}

// ---------------------------------------------------------------------------
// SP-1: State save — enable/disable persists to extension-states.json
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sp1_state_saved_after_enable_disable() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");
    let store = ExtensionStateStore::new(path.clone());

    store.load().await.unwrap();

    // Enable an extension.
    store.set(make_state("my-ext", "1.0.0", true)).await;

    // Disable another.
    store.set(make_state("other-ext", "2.0.0", false)).await;

    // Flush to disk immediately.
    store.flush().await.unwrap();

    // Verify the file contains both states.
    let loaded = load_states_from_file(&path).unwrap();
    assert_eq!(loaded.len(), 2);

    assert!(loaded["my-ext"].enabled);
    assert_eq!(loaded["my-ext"].version, "1.0.0");

    assert!(!loaded["other-ext"].enabled);
    assert_eq!(loaded["other-ext"].version, "2.0.0");
}

#[tokio::test]
async fn sp1_state_contains_timestamps() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");
    let store = ExtensionStateStore::new(path.clone());

    store.load().await.unwrap();
    store.set(make_state("my-ext", "1.0.0", true)).await;
    store.flush().await.unwrap();

    let loaded = load_states_from_file(&path).unwrap();
    assert!(loaded["my-ext"].installed_at.is_some());
}

// ---------------------------------------------------------------------------
// SP-2: State restore — app restart restores previous enabled/disabled state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sp2_state_restored_after_restart() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");

    // "First session": save states.
    {
        let store = ExtensionStateStore::new(path.clone());
        store.load().await.unwrap();
        store.set(make_state("ext-a", "1.0.0", true)).await;
        store.set(make_state("ext-b", "2.0.0", false)).await;
        store.flush().await.unwrap();
    }

    // "Second session": load from the same file.
    {
        let store = ExtensionStateStore::new(path.clone());
        let states = store.load().await.unwrap();

        assert_eq!(states.len(), 2);
        assert!(states["ext-a"].enabled);
        assert!(!states["ext-b"].enabled);

        // Also verify in-memory read.
        let a = store.get("ext-a").await.unwrap();
        assert!(a.enabled);
        assert_eq!(a.version, "1.0.0");

        let b = store.get("ext-b").await.unwrap();
        assert!(!b.enabled);
        assert_eq!(b.version, "2.0.0");
    }
}

// ---------------------------------------------------------------------------
// SP-3: No state file on first launch → all extensions default to enabled
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sp3_no_state_file_returns_empty_map() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("nonexistent-states.json");
    let store = ExtensionStateStore::new(path);

    let states = store.load().await.unwrap();
    assert!(states.is_empty());
    // An empty map means no overrides — the loader creates extensions with
    // enabled=true by default (verified in extension_loading_test.rs).
}

#[test]
fn sp3_load_states_from_nonexistent_file_returns_empty() {
    let states = load_states_from_file(std::path::Path::new("/nonexistent/states.json")).unwrap();
    assert!(states.is_empty());
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_update_overwrites_previous() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");
    let store = ExtensionStateStore::new(path.clone());

    store.load().await.unwrap();

    // Enable, then disable.
    store.set(make_state("my-ext", "1.0.0", true)).await;
    store.set(make_state("my-ext", "1.0.0", false)).await;

    store.flush().await.unwrap();

    let loaded = load_states_from_file(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(!loaded["my-ext"].enabled);
}

#[tokio::test]
async fn set_all_replaces_entire_state() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");
    let store = ExtensionStateStore::new(path.clone());

    store.load().await.unwrap();
    store.set(make_state("old-ext", "1.0.0", true)).await;

    let mut new_states = HashMap::new();
    new_states.insert("new-ext".to_string(), make_state("new-ext", "2.0.0", false));
    store.set_all(new_states).await;

    store.flush().await.unwrap();

    let loaded = load_states_from_file(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("new-ext"));
    assert!(!loaded.contains_key("old-ext"));
}

#[tokio::test]
async fn remove_state() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("extension-states.json");
    let store = ExtensionStateStore::new(path.clone());

    store.load().await.unwrap();
    store.set(make_state("ext-a", "1.0.0", true)).await;
    store.set(make_state("ext-b", "1.0.0", true)).await;
    store.remove("ext-a").await;
    store.flush().await.unwrap();

    let loaded = load_states_from_file(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("ext-b"));
}

#[test]
fn save_and_load_file_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("states.json");

    let mut states = HashMap::new();
    states.insert("ext-a".to_string(), make_state("ext-a", "1.0.0", true));
    states.insert("ext-b".to_string(), make_state("ext-b", "2.0.0", false));

    save_states_to_file(&path, &states).unwrap();
    let loaded = load_states_from_file(&path).unwrap();

    assert_eq!(loaded.len(), 2);
    assert!(loaded["ext-a"].enabled);
    assert!(!loaded["ext-b"].enabled);
}

#[test]
fn save_creates_parent_dirs() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("nested").join("dir").join("states.json");

    save_states_to_file(&path, &HashMap::new()).unwrap();
    assert!(path.exists());
}
