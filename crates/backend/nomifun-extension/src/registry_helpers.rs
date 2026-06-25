use std::collections::HashMap;
use std::path::Path;

use tracing::{debug, warn};

use crate::dependency::{DependencyValidationResult, validate_dependencies};
use crate::lifecycle::{HookKind, execute_hook, resolve_hook_path};
use crate::loader::{ScanPath, filter_by_engine_compatibility, load_all};
use crate::types::{ExtensionSource, ExtensionState, LoadedExtension};

// ---------------------------------------------------------------------------
// ExtensionSummary
// ---------------------------------------------------------------------------

/// Lightweight summary of a loaded extension.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ExtensionSummary {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub source: ExtensionSource,
}

pub(crate) fn to_summary(ext: &LoadedExtension) -> ExtensionSummary {
    ExtensionSummary {
        name: ext.manifest.name.clone(),
        version: ext.manifest.version.clone(),
        display_name: ext.manifest.display_name.clone(),
        description: ext.manifest.description.clone(),
        enabled: ext.state.enabled,
        source: ext.source,
    }
}

// ---------------------------------------------------------------------------
// Load + validate pipeline
// ---------------------------------------------------------------------------

/// Load extensions, filter by engine compatibility, validate dependencies, and
/// sort by topological order. Returns the sorted extensions and the validation
/// result.
pub(crate) fn load_and_validate(
    scan_paths: &[ScanPath],
    app_version: &str,
) -> (Vec<LoadedExtension>, DependencyValidationResult) {
    let loaded = load_all(scan_paths);
    debug!(count = loaded.len(), "loaded extension manifests");

    let filtered = filter_by_engine_compatibility(loaded, app_version);
    debug!(count = filtered.len(), "after engine compatibility filter");

    let dep_result = validate_dependencies(&filtered);
    let sorted = sort_by_load_order(filtered, &dep_result.load_order);
    debug!(count = sorted.len(), "after dependency sort");

    (sorted, dep_result)
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Reorder extensions according to the given load order.
///
/// Extensions not in `load_order` are appended at the end in alphabetical
/// order.
pub(crate) fn sort_by_load_order(extensions: Vec<LoadedExtension>, load_order: &[String]) -> Vec<LoadedExtension> {
    let mut by_name: HashMap<String, LoadedExtension> =
        extensions.into_iter().map(|e| (e.manifest.name.clone(), e)).collect();

    let mut sorted = Vec::with_capacity(by_name.len());

    // First, add extensions in load_order.
    for name in load_order {
        if let Some(ext) = by_name.remove(name) {
            sorted.push(ext);
        }
    }

    // Append any remaining (not in load_order) in alphabetical order.
    let mut remaining: Vec<LoadedExtension> = by_name.into_values().collect();
    remaining.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    sorted.extend(remaining);

    sorted
}

// ---------------------------------------------------------------------------
// State merging + building
// ---------------------------------------------------------------------------

/// Merge persisted enabled/disabled states into freshly loaded extensions.
///
/// If no persisted state exists for an extension, it defaults to enabled.
pub(crate) fn merge_persisted_states(
    mut extensions: Vec<LoadedExtension>,
    persisted: &HashMap<String, ExtensionState>,
) -> Vec<LoadedExtension> {
    for ext in &mut extensions {
        if let Some(saved) = persisted.get(&ext.manifest.name) {
            ext.state.enabled = saved.enabled;
            ext.state.installed_at = saved.installed_at;
            ext.state.last_activated_at = saved.last_activated_at;
        }
    }
    extensions
}

/// Build a state map from the current extensions for persistence.
pub(crate) fn build_state_map(extensions: &[LoadedExtension]) -> HashMap<String, ExtensionState> {
    extensions
        .iter()
        .map(|e| (e.state.name.clone(), e.state.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Deactivation hooks
// ---------------------------------------------------------------------------

/// Run `onDeactivate` hooks for all enabled extensions.
///
/// Errors are logged but do not propagate.
pub(crate) async fn run_deactivation_hooks(extensions: &[LoadedExtension]) {
    for ext in extensions {
        if !ext.state.enabled {
            continue;
        }

        let Some(hooks) = &ext.manifest.lifecycle else {
            continue;
        };
        let Some(hook_path) = resolve_hook_path(hooks, HookKind::OnDeactivate) else {
            continue;
        };

        let ext_dir = Path::new(&ext.directory);
        if let Err(e) = execute_hook(ext_dir, hook_path, HookKind::OnDeactivate, &ext.manifest.name).await {
            warn!(
                extension = %ext.manifest.name,
                error = %e,
                "onDeactivate hook failed during hot reload"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExtensionManifest, ExtensionSource, ExtensionState};

    fn make_test_ext(name: &str, enabled: bool) -> LoadedExtension {
        LoadedExtension {
            manifest: ExtensionManifest {
                name: name.to_owned(),
                version: "1.0.0".to_owned(),
                display_name: Some(format!("{name} Display")),
                description: Some(format!("{name} description")),
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
            directory: format!("/tmp/ext/{name}"),
            source: ExtensionSource::Local,
            state: ExtensionState {
                name: name.to_owned(),
                version: "1.0.0".to_owned(),
                enabled,
                installed_at: Some(1000),
                last_activated_at: None,
            },
        }
    }

    // -- sort_by_load_order ---------------------------------------------------

    #[test]
    fn sort_respects_load_order() {
        let exts = vec![
            make_test_ext("ext-c", true),
            make_test_ext("ext-a", true),
            make_test_ext("ext-b", true),
        ];
        let order = vec!["ext-a".to_owned(), "ext-b".to_owned(), "ext-c".to_owned()];
        let sorted = sort_by_load_order(exts, &order);
        let names: Vec<&str> = sorted.iter().map(|e| e.manifest.name.as_str()).collect();
        assert_eq!(names, vec!["ext-a", "ext-b", "ext-c"]);
    }

    #[test]
    fn sort_appends_unordered_extensions() {
        let exts = vec![
            make_test_ext("ext-z", true),
            make_test_ext("ext-a", true),
            make_test_ext("ext-m", true),
        ];
        // Only ext-a is in load order
        let order = vec!["ext-a".to_owned()];
        let sorted = sort_by_load_order(exts, &order);
        let names: Vec<&str> = sorted.iter().map(|e| e.manifest.name.as_str()).collect();
        assert_eq!(names, vec!["ext-a", "ext-m", "ext-z"]);
    }

    #[test]
    fn sort_empty_load_order() {
        let exts = vec![make_test_ext("ext-b", true), make_test_ext("ext-a", true)];
        let sorted = sort_by_load_order(exts, &[]);
        let names: Vec<&str> = sorted.iter().map(|e| e.manifest.name.as_str()).collect();
        assert_eq!(names, vec!["ext-a", "ext-b"]);
    }

    // -- merge_persisted_states ------------------------------------------------

    #[test]
    fn merge_applies_persisted_enabled() {
        let exts = vec![make_test_ext("ext-a", true)];
        let mut persisted = HashMap::new();
        persisted.insert(
            "ext-a".to_owned(),
            ExtensionState {
                name: "ext-a".to_owned(),
                version: "1.0.0".to_owned(),
                enabled: false,
                installed_at: Some(500),
                last_activated_at: Some(600),
            },
        );

        let merged = merge_persisted_states(exts, &persisted);
        assert!(!merged[0].state.enabled);
        assert_eq!(merged[0].state.installed_at, Some(500));
        assert_eq!(merged[0].state.last_activated_at, Some(600));
    }

    #[test]
    fn merge_defaults_to_enabled_when_no_persisted() {
        let exts = vec![make_test_ext("ext-a", true)];
        let merged = merge_persisted_states(exts, &HashMap::new());
        assert!(merged[0].state.enabled);
    }

    // -- build_state_map ------------------------------------------------------

    #[test]
    fn build_state_map_includes_all_extensions() {
        let exts = vec![make_test_ext("ext-a", true), make_test_ext("ext-b", false)];
        let map = build_state_map(&exts);
        assert_eq!(map.len(), 2);
        assert!(map["ext-a"].enabled);
        assert!(!map["ext-b"].enabled);
    }

    // -- to_summary -----------------------------------------------------------

    #[test]
    fn summary_maps_fields_correctly() {
        let ext = make_test_ext("my-ext", true);
        let summary = to_summary(&ext);
        assert_eq!(summary.name, "my-ext");
        assert_eq!(summary.version, "1.0.0");
        assert_eq!(summary.display_name.as_deref(), Some("my-ext Display"));
        assert!(summary.enabled);
        assert_eq!(summary.source, ExtensionSource::Local);
    }
}
