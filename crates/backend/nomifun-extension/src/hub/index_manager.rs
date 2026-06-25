use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::constants::HUB_SUPPORTED_SCHEMA_VERSION;
use crate::error::ExtensionError;
use crate::registry::ExtensionRegistry;
use crate::types::{HubExtensionStatus, HubExtensionWithStatus};

// ---------------------------------------------------------------------------
// Hub index on-disk format
// ---------------------------------------------------------------------------

/// Schema envelope for a Hub index file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HubIndexFile {
    /// Schema version — we only support [`HUB_SUPPORTED_SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    /// Extension entries in the index.
    #[serde(default)]
    extensions: Vec<HubIndexEntry>,
}

fn default_schema_version() -> u32 {
    1
}

/// A single entry in the Hub index file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HubIndexEntry {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Whether this extension is bundled with the app (no download needed).
    #[serde(default)]
    pub bundled: bool,
    /// Optional download URL for remote extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}

// ---------------------------------------------------------------------------
// HubIndexManager
// ---------------------------------------------------------------------------

/// Manages the Hub extension index — loads from local file, merges
/// install status from the live extension registry.
#[derive(Clone)]
pub struct HubIndexManager {
    /// Directory that contains `index.json`.
    index_dir: PathBuf,
    /// Reference to the live extension registry for status resolution.
    registry: ExtensionRegistry,
}

impl HubIndexManager {
    /// Create a new index manager.
    ///
    /// - `index_dir`: directory containing the Hub `index.json`.
    /// - `registry`: live extension registry used to determine install status.
    pub fn new(index_dir: PathBuf, registry: ExtensionRegistry) -> Self {
        Self { index_dir, registry }
    }

    /// Load the Hub index and merge install status from the registry.
    ///
    /// Returns a list of extensions with their current status.
    pub async fn load_index(&self) -> Vec<HubExtensionWithStatus> {
        let entries = self.load_index_entries();
        self.merge_with_registry_status(entries).await
    }

    /// Look up a single extension by name from the index.
    pub(crate) fn get_extension(&self, name: &str) -> Option<HubIndexEntry> {
        let entries = self.load_index_entries();
        entries.into_iter().find(|e| e.name == name)
    }

    /// Return the directory where extensions should be installed.
    pub fn install_target_dir(&self) -> PathBuf {
        self.index_dir.clone()
    }

    /// Return the index file path.
    fn index_file_path(&self) -> PathBuf {
        self.index_dir.join("index.json")
    }

    /// Load index entries from disk, falling back to an empty list.
    fn load_index_entries(&self) -> Vec<HubIndexEntry> {
        let path = self.index_file_path();
        match load_index_from_file(&path) {
            Ok(entries) => entries,
            Err(e) => {
                debug!(
                    path = %path.display(),
                    error = %e,
                    "hub index not found or invalid, returning empty list"
                );
                Vec::new()
            }
        }
    }

    /// Merge index entries with live registry status.
    async fn merge_with_registry_status(&self, entries: Vec<HubIndexEntry>) -> Vec<HubExtensionWithStatus> {
        let loaded = self.registry.get_loaded_extensions().await;
        let installed: HashMap<String, String> = loaded.into_iter().map(|s| (s.name, s.version)).collect();

        entries
            .into_iter()
            .map(|entry| {
                let status = resolve_status(&entry, &installed);
                HubExtensionWithStatus {
                    name: entry.name,
                    version: entry.version,
                    display_name: entry.display_name,
                    description: entry.description,
                    author: entry.author,
                    icon: entry.icon,
                    tags: entry.tags,
                    bundled: entry.bundled,
                    status,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Index file I/O
// ---------------------------------------------------------------------------

/// Read and parse the Hub index file, returning entries.
fn load_index_from_file(path: &Path) -> Result<Vec<HubIndexEntry>, ExtensionError> {
    let bytes = std::fs::read(path)?;
    let index: HubIndexFile = serde_json::from_slice(&bytes)?;

    if index.schema_version != HUB_SUPPORTED_SCHEMA_VERSION {
        warn!(
            found = index.schema_version,
            expected = HUB_SUPPORTED_SCHEMA_VERSION,
            "hub index schema version mismatch — attempting best-effort parse"
        );
    }

    Ok(index.extensions)
}

// ---------------------------------------------------------------------------
// Status resolution
// ---------------------------------------------------------------------------

/// Determine the runtime status of a Hub entry by checking whether
/// it is loaded in the registry.
fn resolve_status(entry: &HubIndexEntry, installed: &HashMap<String, String>) -> HubExtensionStatus {
    if entry.bundled {
        return HubExtensionStatus::Installed;
    }

    match installed.get(&entry.name) {
        Some(installed_version) => {
            if is_update_available(&entry.version, installed_version) {
                HubExtensionStatus::UpdateAvailable
            } else {
                HubExtensionStatus::Installed
            }
        }
        None => HubExtensionStatus::NotInstalled,
    }
}

/// Check if the index version is newer than the installed version.
fn is_update_available(index_version: &str, installed_version: &str) -> bool {
    let Ok(idx) = semver::Version::parse(index_version) else {
        return false;
    };
    let Ok(inst) = semver::Version::parse(installed_version) else {
        return false;
    };
    idx > inst
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_status_bundled_always_installed() {
        let entry = HubIndexEntry {
            name: "builtin-ext".into(),
            version: "1.0.0".into(),
            display_name: None,
            description: None,
            author: None,
            icon: None,
            tags: Vec::new(),
            bundled: true,
            download_url: None,
        };
        let installed = HashMap::new();
        assert_eq!(resolve_status(&entry, &installed), HubExtensionStatus::Installed);
    }

    #[test]
    fn resolve_status_not_installed() {
        let entry = HubIndexEntry {
            name: "new-ext".into(),
            version: "1.0.0".into(),
            display_name: None,
            description: None,
            author: None,
            icon: None,
            tags: Vec::new(),
            bundled: false,
            download_url: None,
        };
        let installed = HashMap::new();
        assert_eq!(resolve_status(&entry, &installed), HubExtensionStatus::NotInstalled);
    }

    #[test]
    fn resolve_status_installed_same_version() {
        let entry = HubIndexEntry {
            name: "my-ext".into(),
            version: "1.0.0".into(),
            display_name: None,
            description: None,
            author: None,
            icon: None,
            tags: Vec::new(),
            bundled: false,
            download_url: None,
        };
        let installed = HashMap::from([("my-ext".into(), "1.0.0".into())]);
        assert_eq!(resolve_status(&entry, &installed), HubExtensionStatus::Installed);
    }

    #[test]
    fn resolve_status_update_available() {
        let entry = HubIndexEntry {
            name: "my-ext".into(),
            version: "2.0.0".into(),
            display_name: None,
            description: None,
            author: None,
            icon: None,
            tags: Vec::new(),
            bundled: false,
            download_url: None,
        };
        let installed = HashMap::from([("my-ext".into(), "1.0.0".into())]);
        assert_eq!(resolve_status(&entry, &installed), HubExtensionStatus::UpdateAvailable);
    }

    #[test]
    fn resolve_status_installed_newer_than_index() {
        let entry = HubIndexEntry {
            name: "my-ext".into(),
            version: "1.0.0".into(),
            display_name: None,
            description: None,
            author: None,
            icon: None,
            tags: Vec::new(),
            bundled: false,
            download_url: None,
        };
        let installed = HashMap::from([("my-ext".into(), "2.0.0".into())]);
        // Installed version is newer — still "installed", not "update_available".
        assert_eq!(resolve_status(&entry, &installed), HubExtensionStatus::Installed);
    }

    #[test]
    fn is_update_available_newer() {
        assert!(is_update_available("2.0.0", "1.0.0"));
    }

    #[test]
    fn is_update_available_same() {
        assert!(!is_update_available("1.0.0", "1.0.0"));
    }

    #[test]
    fn is_update_available_older() {
        assert!(!is_update_available("1.0.0", "2.0.0"));
    }

    #[test]
    fn is_update_available_invalid_version() {
        assert!(!is_update_available("not-semver", "1.0.0"));
        assert!(!is_update_available("1.0.0", "not-semver"));
    }

    #[test]
    fn load_index_from_file_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index = HubIndexFile {
            schema_version: 1,
            extensions: vec![HubIndexEntry {
                name: "test-ext".into(),
                version: "1.0.0".into(),
                display_name: Some("Test Extension".into()),
                description: Some("A test extension".into()),
                author: Some("Test Author".into()),
                icon: None,
                tags: vec!["tools".into()],
                bundled: false,
                download_url: Some("https://example.com/test-ext-1.0.0.tar.gz".into()),
            }],
        };
        let path = tmp.path().join("index.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

        let entries = load_index_from_file(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "test-ext");
        assert_eq!(entries[0].version, "1.0.0");
        assert!(!entries[0].bundled);
    }

    #[test]
    fn load_index_from_file_not_found() {
        let result = load_index_from_file(Path::new("/nonexistent/index.json"));
        assert!(result.is_err());
    }

    #[test]
    fn load_index_from_file_invalid_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("index.json");
        std::fs::write(&path, b"not valid json").unwrap();

        let result = load_index_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_index_from_file_empty_extensions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index = HubIndexFile {
            schema_version: 1,
            extensions: Vec::new(),
        };
        let path = tmp.path().join("index.json");
        std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();

        let entries = load_index_from_file(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn hub_index_entry_deserialization() {
        let json = serde_json::json!({
            "name": "my-ext",
            "version": "2.0.0",
            "display_name": "My Extension",
            "tags": ["ai", "tools"],
            "bundled": true
        });
        let entry: HubIndexEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.name, "my-ext");
        assert_eq!(entry.version, "2.0.0");
        assert_eq!(entry.display_name.as_deref(), Some("My Extension"));
        assert_eq!(entry.tags, vec!["ai", "tools"]);
        assert!(entry.bundled);
        assert!(entry.download_url.is_none());
    }
}
