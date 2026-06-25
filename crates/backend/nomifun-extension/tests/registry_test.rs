//! Integration tests for the extension registry (test-plan EM-1..EM-4, HR-3,
//! EQ-1..EQ-2, SP-1..SP-3 at registry level).
//!
//! These tests exercise `ExtensionRegistry` as a black box: initialization,
//! enable/disable with event broadcasting, hot-reload sequence, and state
//! persistence through the registry API.
//!
//! All tests use `initialize_with_scan_paths` with explicit paths to avoid
//! process-level env var races when running in parallel.

use std::collections::HashMap;
use std::sync::Arc;

use nomifun_extension::{
    ExtensionManifest, ExtensionRegistry, ExtensionSource, ExtensionState, ExtensionStateStore, LoadedExtension,
    ScanPath, save_states_to_file,
};
use nomifun_realtime::BroadcastEventBus;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_ext(name: &str, version: &str, enabled: bool) -> LoadedExtension {
    LoadedExtension {
        manifest: ExtensionManifest {
            name: name.to_owned(),
            version: version.to_owned(),
            display_name: Some(format!("{name} Display")),
            description: Some(format!("Description for {name}")),
            author: None,
            license: None,
            homepage: None,
            icon: None,
            engine: None,
            api_version: None,
            dependencies: HashMap::new(),
            entry_point: None,
            permissions: None,
            contributes: None,
            lifecycle: None,
            i18n: None,
        },
        directory: format!("/tmp/test-ext/{name}"),
        source: ExtensionSource::Local,
        state: ExtensionState {
            name: name.to_owned(),
            version: version.to_owned(),
            enabled,
            installed_at: Some(1_700_000_000_000),
            last_activated_at: None,
        },
    }
}

/// Write extension fixture files to `ext_dir` and return scan paths for them.
fn write_fixtures(tmp: &TempDir, extensions: &[LoadedExtension]) -> (std::path::PathBuf, Vec<ScanPath>) {
    let ext_dir = tmp.path().join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    for ext in extensions {
        let dir = ext_dir.join(&ext.manifest.name);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "name": ext.manifest.name,
            "version": ext.manifest.version,
            "display_name": ext.manifest.display_name,
            "description": ext.manifest.description,
        });
        std::fs::write(
            dir.join("nomi-extension.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    let scan_paths = vec![ScanPath {
        path: ext_dir.clone(),
        source: ExtensionSource::Env,
    }];
    (ext_dir, scan_paths)
}

/// Create a registry pre-seeded with extensions (bypasses env var resolution).
async fn seeded_registry(extensions: Vec<LoadedExtension>) -> (ExtensionRegistry, Arc<BroadcastEventBus>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = ExtensionStateStore::new(tmp.path().join("states.json"));
    let bus = Arc::new(BroadcastEventBus::new(64));
    let registry = ExtensionRegistry::new(store, bus.clone(), "1.0.0".to_owned());

    let (_, scan_paths) = write_fixtures(&tmp, &extensions);

    // Pre-populate state file for disabled extensions.
    let states: HashMap<String, ExtensionState> = extensions
        .iter()
        .map(|e| (e.state.name.clone(), e.state.clone()))
        .collect();
    save_states_to_file(&tmp.path().join("states.json"), &states).unwrap();

    // Initialize using explicit scan paths — no env vars needed.
    registry.initialize_with_scan_paths(scan_paths).await.unwrap();

    (registry, bus, tmp)
}

// ---------------------------------------------------------------------------
// EQ-1: Get loaded extensions — empty
// ---------------------------------------------------------------------------

#[tokio::test]
async fn eq1_get_loaded_extensions_empty() {
    let tmp = TempDir::new().unwrap();
    let store = ExtensionStateStore::new(tmp.path().join("states.json"));
    let bus = Arc::new(BroadcastEventBus::new(16));
    let registry = ExtensionRegistry::new(store, bus, "1.0.0".to_owned());

    // Empty directory — no extensions to load.
    let empty_dir = tmp.path().join("empty-exts");
    std::fs::create_dir_all(&empty_dir).unwrap();

    let scan_paths = vec![ScanPath {
        path: empty_dir,
        source: ExtensionSource::Env,
    }];
    registry.initialize_with_scan_paths(scan_paths).await.unwrap();

    let exts = registry.get_loaded_extensions().await;
    assert!(exts.is_empty(), "expected empty extension list");
}

// ---------------------------------------------------------------------------
// EQ-2: Get loaded extensions — with extensions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn eq2_get_loaded_extensions_with_content() {
    let extensions = vec![
        make_ext("ext-alpha", "1.0.0", true),
        make_ext("ext-beta", "2.0.0", true),
    ];

    let (registry, _, _tmp) = seeded_registry(extensions).await;

    let summaries = registry.get_loaded_extensions().await;
    assert_eq!(summaries.len(), 2);

    let names: Vec<&str> = summaries.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"ext-alpha"));
    assert!(names.contains(&"ext-beta"));

    // Verify summary fields.
    let alpha = summaries.iter().find(|s| s.name == "ext-alpha").unwrap();
    assert_eq!(alpha.version, "1.0.0");
    assert!(alpha.enabled);
    assert_eq!(alpha.display_name.as_deref(), Some("ext-alpha Display"));
}

// ---------------------------------------------------------------------------
// EM-1: Enable extension → stateChanged event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn em1_enable_extension_broadcasts_state_changed() {
    let extensions = vec![make_ext("my-ext", "1.0.0", false)];
    let (registry, bus, _tmp) = seeded_registry(extensions).await;

    let mut rx = bus.subscribe();

    registry.enable_extension("my-ext").await.unwrap();

    // Verify the extension is now enabled.
    let ext = registry.get_extension_by_name("my-ext").await.unwrap();
    assert!(ext.state.enabled);
    assert!(ext.state.last_activated_at.is_some());

    // Verify stateChanged event was broadcast.
    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.name, "extensions.state-changed");
    assert_eq!(msg.data["name"], "my-ext");
    assert_eq!(msg.data["enabled"], true);
}

// ---------------------------------------------------------------------------
// EM-2: Disable extension with reason → stateChanged event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn em2_disable_extension_broadcasts_state_changed() {
    let extensions = vec![make_ext("my-ext", "1.0.0", true)];
    let (registry, bus, _tmp) = seeded_registry(extensions).await;

    let mut rx = bus.subscribe();

    registry
        .disable_extension("my-ext", Some("Security concern"))
        .await
        .unwrap();

    // Verify the extension is now disabled.
    let ext = registry.get_extension_by_name("my-ext").await.unwrap();
    assert!(!ext.state.enabled);

    // Verify stateChanged event was broadcast.
    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.name, "extensions.state-changed");
    assert_eq!(msg.data["name"], "my-ext");
    assert_eq!(msg.data["enabled"], false);
}

// ---------------------------------------------------------------------------
// EM-3: Enable non-existent extension → error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn em3_enable_nonexistent_returns_error() {
    let (registry, _, _tmp) = seeded_registry(vec![]).await;
    let result = registry.enable_extension("nonexistent").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// EM-4: Disable non-existent extension → error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn em4_disable_nonexistent_returns_error() {
    let (registry, _, _tmp) = seeded_registry(vec![]).await;
    let result = registry.disable_extension("nonexistent", None).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// HR-3: Hot-reload sequence — deactivate → clear → reload → event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hr3_hot_reload_emits_registry_reloaded() {
    let extensions = vec![make_ext("test-ext", "1.0.0", true)];
    let (registry, bus, _tmp) = seeded_registry(extensions).await;

    let mut rx = bus.subscribe();

    registry.hot_reload().await;

    // Should receive at least one lifecycle event for REGISTRY_RELOADED.
    // Drain events until we find it (there may be EXTENSION_ACTIVATED events first).
    let mut found_reload = false;
    while let Ok(msg) = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
        if let Ok(msg) = msg
            && msg.name == "extensions.lifecycle"
            && msg.data["event"] == "REGISTRY_RELOADED"
        {
            found_reload = true;
            break;
        }
    }
    assert!(found_reload, "expected REGISTRY_RELOADED event");

    // Extensions should still be loaded after reload.
    let exts = registry.get_loaded_extensions().await;
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name, "test-ext");
}

// ---------------------------------------------------------------------------
// HR-3 (continued): Hot-reload preserves enabled/disabled state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hr3_hot_reload_preserves_disabled_state() {
    let extensions = vec![
        make_ext("enabled-ext", "1.0.0", true),
        make_ext("disabled-ext", "1.0.0", false),
    ];
    let (registry, _, _tmp) = seeded_registry(extensions).await;

    // Disable an extension, then hot-reload.
    registry.disable_extension("enabled-ext", None).await.unwrap();

    registry.hot_reload().await;

    let exts = registry.get_loaded_extensions().await;
    let enabled = exts.iter().find(|e| e.name == "enabled-ext").unwrap();
    assert!(!enabled.enabled, "disabled state should persist through reload");
}

// ---------------------------------------------------------------------------
// SP: State persistence through registry enable/disable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sp_enable_disable_persists_through_reload() {
    let tmp = TempDir::new().unwrap();
    let state_path = tmp.path().join("states.json");
    let store = ExtensionStateStore::new(state_path.clone());
    let bus = Arc::new(BroadcastEventBus::new(16));

    // Create fixture extensions.
    let ext_dir = tmp.path().join("extensions");
    let ext_a_dir = ext_dir.join("ext-a");
    std::fs::create_dir_all(&ext_a_dir).unwrap();
    std::fs::write(
        ext_a_dir.join("nomi-extension.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "ext-a",
            "version": "1.0.0",
        }))
        .unwrap(),
    )
    .unwrap();

    let scan_paths = vec![ScanPath {
        path: ext_dir,
        source: ExtensionSource::Env,
    }];

    let registry = ExtensionRegistry::new(store.clone(), bus, "1.0.0".to_owned());
    registry.initialize_with_scan_paths(scan_paths.clone()).await.unwrap();

    // Extension should be enabled by default (SP-3).
    let exts = registry.get_loaded_extensions().await;
    assert!(exts[0].enabled, "first load should default to enabled");

    // Disable it.
    registry.disable_extension("ext-a", None).await.unwrap();

    // Flush to disk.
    store.flush().await.unwrap();

    // Verify persisted state (SP-1).
    let loaded = nomifun_extension::load_states_from_file(&state_path).unwrap();
    assert!(!loaded["ext-a"].enabled);

    // Re-initialize (simulate restart) — should restore disabled state (SP-2).
    let store2 = ExtensionStateStore::new(state_path);
    let bus2 = Arc::new(BroadcastEventBus::new(16));
    let registry2 = ExtensionRegistry::new(store2, bus2, "1.0.0".to_owned());
    registry2.initialize_with_scan_paths(scan_paths).await.unwrap();

    let exts2 = registry2.get_loaded_extensions().await;
    assert!(!exts2.is_empty(), "second registry should find extensions");
    assert!(!exts2[0].enabled, "disabled state should persist across restarts");
}

// ---------------------------------------------------------------------------
// Query: contribution getters return empty on no extensions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn query_empty_contributions_on_no_extensions() {
    let (registry, _, _tmp) = seeded_registry(vec![]).await;

    assert!(registry.get_themes().await.is_empty());
    assert!(registry.get_assistants().await.is_empty());
    assert!(registry.get_acp_adapters().await.is_empty());
    assert!(registry.get_agents().await.is_empty());
    assert!(registry.get_mcp_servers().await.is_empty());
    assert!(registry.get_skills().await.is_empty());
    assert!(registry.get_settings_tabs().await.is_empty());
    assert!(registry.get_webui_contributions().await.is_empty());
    assert!(registry.get_channel_plugins().await.is_empty());
    assert!(registry.get_model_providers().await.is_empty());
}

// ---------------------------------------------------------------------------
// Enable idempotent — enabling an already-enabled extension is a no-op
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enable_already_enabled_is_noop() {
    let extensions = vec![make_ext("my-ext", "1.0.0", true)];
    let (registry, bus, _tmp) = seeded_registry(extensions).await;

    let mut rx = bus.subscribe();

    // Enable an already-enabled extension — should not broadcast.
    registry.enable_extension("my-ext").await.unwrap();

    // No stateChanged event expected — timeout means success.
    let result = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(result.is_err(), "no event expected for no-op enable");
}

// ---------------------------------------------------------------------------
// Initialize sets initialized flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn initialize_sets_flag() {
    let tmp = TempDir::new().unwrap();
    let store = ExtensionStateStore::new(tmp.path().join("states.json"));
    let bus = Arc::new(BroadcastEventBus::new(16));
    let registry = ExtensionRegistry::new(store, bus, "1.0.0".to_owned());

    assert!(!registry.is_initialized().await);

    let empty_dir = tmp.path().join("empty-exts");
    std::fs::create_dir_all(&empty_dir).unwrap();

    let scan_paths = vec![ScanPath {
        path: empty_dir,
        source: ExtensionSource::Env,
    }];
    registry.initialize_with_scan_paths(scan_paths).await.unwrap();

    assert!(registry.is_initialized().await);
}
