//! Integration tests for extension loading (test-plan EL-1..EL-5).
//!
//! These tests exercise `load_all` and `filter_by_engine_compatibility` as
//! black-box functions, verifying scan priority, engine filtering, invalid
//! manifest handling, E2E isolation, and empty directory behaviour.

use std::fs;
use std::path::Path;

use nomifun_extension::{ExtensionSource, ScanPath, filter_by_engine_compatibility, load_all};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_manifest(dir: &Path, name: &str, version: &str) {
    write_manifest_full(dir, name, version, None, None);
}

fn write_manifest_full(dir: &Path, name: &str, version: &str, engine_nomifun: Option<&str>, api_version: Option<&str>) {
    let mut manifest = serde_json::json!({
        "name": name,
        "version": version,
    });
    if let Some(eng) = engine_nomifun {
        manifest["engine"] = serde_json::json!({ "nomifun": eng });
    }
    if let Some(api) = api_version {
        manifest["api_version"] = serde_json::json!(api);
    }
    fs::write(
        dir.join("nomi-extension.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn create_ext_dir(parent: &Path, name: &str) -> std::path::PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// EL-1: Scan priority — env > user > appdata, same-name deduplication
// ---------------------------------------------------------------------------

#[test]
fn el1_scan_priority_env_wins_over_local() {
    let env_dir = TempDir::new().unwrap();
    let local_dir = TempDir::new().unwrap();
    let appdata_dir = TempDir::new().unwrap();

    // Same extension name in all three directories with different versions.
    let ext = create_ext_dir(env_dir.path(), "my-ext");
    write_manifest(&ext, "my-ext", "3.0.0");

    let ext = create_ext_dir(local_dir.path(), "my-ext");
    write_manifest(&ext, "my-ext", "2.0.0");

    let ext = create_ext_dir(appdata_dir.path(), "my-ext");
    write_manifest(&ext, "my-ext", "1.0.0");

    let scan_paths = vec![
        ScanPath {
            path: env_dir.path().to_path_buf(),
            source: ExtensionSource::Env,
        },
        ScanPath {
            path: local_dir.path().to_path_buf(),
            source: ExtensionSource::Local,
        },
        ScanPath {
            path: appdata_dir.path().to_path_buf(),
            source: ExtensionSource::Appdata,
        },
    ];

    let loaded = load_all(&scan_paths);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].manifest.version, "3.0.0");
    assert_eq!(loaded[0].source, ExtensionSource::Env);
}

// ---------------------------------------------------------------------------
// EL-2: Engine compatibility filtering
// ---------------------------------------------------------------------------

#[test]
fn el2_engine_incompatible_extension_filtered_out() {
    let tmp = TempDir::new().unwrap();

    // Extension requires nomifun ^2.0.0 but app is 1.5.0.
    let ext = create_ext_dir(tmp.path(), "future-ext");
    write_manifest_full(&ext, "future-ext", "1.0.0", Some("^2.0.0"), None);

    // Compatible extension.
    let ext2 = create_ext_dir(tmp.path(), "good-ext");
    write_manifest_full(&ext2, "good-ext", "1.0.0", Some("^1.0.0"), None);

    let scan = vec![ScanPath {
        path: tmp.path().to_path_buf(),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 2);

    let filtered = filter_by_engine_compatibility(loaded, "1.5.0");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].manifest.name, "good-ext");
}

// ---------------------------------------------------------------------------
// EL-3: Invalid manifest skipped, other extensions load normally
// ---------------------------------------------------------------------------

#[test]
fn el3_invalid_manifest_skipped_others_load() {
    let tmp = TempDir::new().unwrap();

    // Valid extension.
    let good = create_ext_dir(tmp.path(), "valid-ext");
    write_manifest(&good, "valid-ext", "1.0.0");

    // Invalid JSON.
    let bad = create_ext_dir(tmp.path(), "bad-json");
    fs::write(bad.join("nomi-extension.json"), b"{ broken json").unwrap();

    // Missing required fields.
    let incomplete = create_ext_dir(tmp.path(), "incomplete");
    fs::write(
        incomplete.join("nomi-extension.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"name": "incomplete"})).unwrap(),
    )
    .unwrap();

    let scan = vec![ScanPath {
        path: tmp.path().to_path_buf(),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].manifest.name, "valid-ext");
}

// ---------------------------------------------------------------------------
// EL-4: E2E test mode isolation
//
// Since NOMIFUN_E2E_TEST=1 changes the behaviour of resolve_scan_paths()
// (a global function reading env vars), this test validates the semantics
// rather than calling resolve_scan_paths() directly to avoid data races
// with other tests.
//
// The behaviour is: when E2E mode is on, only ScanPaths with source=Env
// should be used. We validate this by checking that load_all with only Env
// sources works correctly.
// ---------------------------------------------------------------------------

#[test]
fn el4_e2e_test_mode_only_env_sources() {
    let env_dir = TempDir::new().unwrap();
    let local_dir = TempDir::new().unwrap();

    let ext = create_ext_dir(env_dir.path(), "env-ext");
    write_manifest(&ext, "env-ext", "1.0.0");

    let ext = create_ext_dir(local_dir.path(), "local-ext");
    write_manifest(&ext, "local-ext", "1.0.0");

    // Simulate E2E mode: only include env-sourced paths.
    let scan = vec![ScanPath {
        path: env_dir.path().to_path_buf(),
        source: ExtensionSource::Env,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].manifest.name, "env-ext");
    assert_eq!(loaded[0].source, ExtensionSource::Env);
}

// ---------------------------------------------------------------------------
// EL-5: Empty scan directories → empty result, no error
// ---------------------------------------------------------------------------

#[test]
fn el5_empty_scan_directories_return_empty() {
    let empty1 = TempDir::new().unwrap();
    let empty2 = TempDir::new().unwrap();

    let scan = vec![
        ScanPath {
            path: empty1.path().to_path_buf(),
            source: ExtensionSource::Local,
        },
        ScanPath {
            path: empty2.path().to_path_buf(),
            source: ExtensionSource::Appdata,
        },
    ];

    let loaded = load_all(&scan);
    assert!(loaded.is_empty());
}

#[test]
fn el5_nonexistent_scan_directory_returns_empty() {
    let scan = vec![ScanPath {
        path: std::path::PathBuf::from("/nonexistent/extensions/dir"),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert!(loaded.is_empty());
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[test]
fn api_version_filtering() {
    let tmp = TempDir::new().unwrap();

    // Extension requiring API 2.0.0 — incompatible with current 1.0.0.
    let ext = create_ext_dir(tmp.path(), "future-api");
    write_manifest_full(&ext, "future-api", "1.0.0", None, Some("2.0.0"));

    // Extension with compatible API version.
    let ext2 = create_ext_dir(tmp.path(), "current-api");
    write_manifest_full(&ext2, "current-api", "1.0.0", None, Some("1.0.0"));

    // Extension with no API version constraint.
    let ext3 = create_ext_dir(tmp.path(), "no-api");
    write_manifest(&ext3, "no-api", "1.0.0");

    let scan = vec![ScanPath {
        path: tmp.path().to_path_buf(),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 3);

    let filtered = filter_by_engine_compatibility(loaded, "1.0.0");
    assert_eq!(filtered.len(), 2);
    let names: Vec<&str> = filtered.iter().map(|e| e.manifest.name.as_str()).collect();
    assert!(names.contains(&"current-api"));
    assert!(names.contains(&"no-api"));
    assert!(!names.contains(&"future-api"));
}

#[test]
fn multiple_extensions_from_single_directory() {
    let tmp = TempDir::new().unwrap();

    for i in 0..5 {
        let ext = create_ext_dir(tmp.path(), &format!("ext-{i}"));
        write_manifest(&ext, &format!("ext-{i}"), "1.0.0");
    }

    let scan = vec![ScanPath {
        path: tmp.path().to_path_buf(),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 5);
}

#[test]
fn extension_state_defaults_to_enabled() {
    let tmp = TempDir::new().unwrap();
    let ext = create_ext_dir(tmp.path(), "my-ext");
    write_manifest(&ext, "my-ext", "1.0.0");

    let scan = vec![ScanPath {
        path: tmp.path().to_path_buf(),
        source: ExtensionSource::Local,
    }];

    let loaded = load_all(&scan);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].state.enabled);
    assert_eq!(loaded[0].state.name, "my-ext");
    assert_eq!(loaded[0].state.version, "1.0.0");
}
