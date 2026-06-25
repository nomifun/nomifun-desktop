use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::constants::{CUSTOM_SKILL_PATHS_FILE, SKILLS_MARKET_NAME, SKILLS_MARKET_PATH};
use crate::error::ExtensionError;
use crate::skill_service::NamedPath;

/// Persistent storage for custom external skill paths.
///
/// Data is stored in `~/.nomifun/custom-skill-paths.json`.
pub struct ExternalPathsManager {
    file_path: PathBuf,
    paths: RwLock<Vec<PersistedNamedPath>>,
}

/// Serializable named path entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PersistedNamedPath {
    name: String,
    path: String,
}

impl ExternalPathsManager {
    /// Create a new manager that persists to the given data directory.
    ///
    /// Loads existing paths from disk if the file exists.
    pub async fn new(data_dir: &Path) -> Self {
        let file_path = data_dir.join(CUSTOM_SKILL_PATHS_FILE);
        let paths = load_from_file(&file_path).await;

        Self {
            file_path,
            paths: RwLock::new(paths),
        }
    }

    /// Create a manager with an explicit persistence file path.
    ///
    /// Useful for testing.
    pub async fn with_file(file_path: PathBuf) -> Self {
        let paths = load_from_file(&file_path).await;

        Self {
            file_path,
            paths: RwLock::new(paths),
        }
    }

    /// Get all custom external paths.
    pub async fn get_custom_external_paths(&self) -> Vec<NamedPath> {
        let paths = self.paths.read().await;
        paths
            .iter()
            .map(|p| NamedPath {
                name: p.name.clone(),
                path: p.path.clone(),
            })
            .collect()
    }

    /// Add a custom external path.
    ///
    /// If a path with the same value already exists, it is updated with the new name.
    pub async fn add_custom_external_path(&self, name: &str, path: &str) -> Result<(), ExtensionError> {
        let mut paths = self.paths.write().await;

        // Update existing or add new
        if let Some(existing) = paths.iter_mut().find(|p| p.path == path) {
            existing.name = name.to_string();
        } else {
            paths.push(PersistedNamedPath {
                name: name.to_string(),
                path: path.to_string(),
            });
        }

        save_to_file(&self.file_path, &paths).await?;
        debug!(name = %name, path = %path, "added custom external path");
        Ok(())
    }

    /// Remove a custom external path by its path value.
    pub async fn remove_custom_external_path(&self, path: &str) -> Result<(), ExtensionError> {
        let mut paths = self.paths.write().await;
        let before_len = paths.len();
        paths.retain(|p| p.path != path);

        if paths.len() < before_len {
            save_to_file(&self.file_path, &paths).await?;
            debug!(path = %path, "removed custom external path");
        }

        Ok(())
    }

    /// Enable the nomifun skills market by adding it to external paths.
    pub async fn enable_skills_market(&self) -> Result<(), ExtensionError> {
        self.add_custom_external_path(SKILLS_MARKET_NAME, SKILLS_MARKET_PATH)
            .await
    }

    /// Disable the nomifun skills market by removing it from external paths.
    pub async fn disable_skills_market(&self) -> Result<(), ExtensionError> {
        self.remove_custom_external_path(SKILLS_MARKET_PATH).await
    }
}

/// Load paths from the persistence file.
async fn load_from_file(file_path: &Path) -> Vec<PersistedNamedPath> {
    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => match serde_json::from_str::<Vec<PersistedNamedPath>>(&content) {
            Ok(paths) => paths,
            Err(e) => {
                warn!(
                    path = %file_path.display(),
                    error = %e,
                    "failed to parse custom skill paths file, starting fresh"
                );
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warn!(
                path = %file_path.display(),
                error = %e,
                "failed to read custom skill paths file, starting fresh"
            );
            Vec::new()
        }
    }
}

/// Save paths to the persistence file.
async fn save_to_file(file_path: &Path, paths: &[PersistedNamedPath]) -> Result<(), ExtensionError> {
    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let json = serde_json::to_string_pretty(paths)?;
    tokio::fs::write(file_path, json).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Basic CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn new_manager_empty_when_no_file() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        let paths = mgr.get_custom_external_paths().await;
        assert!(paths.is_empty());
    }

    #[tokio::test]
    async fn add_and_get_paths() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.add_custom_external_path("My Skills", "/home/user/skills")
            .await
            .unwrap();
        mgr.add_custom_external_path("Work Skills", "/work/skills")
            .await
            .unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].name, "My Skills");
        assert_eq!(paths[0].path, "/home/user/skills");
        assert_eq!(paths[1].name, "Work Skills");
        assert_eq!(paths[1].path, "/work/skills");
    }

    #[tokio::test]
    async fn add_duplicate_path_updates_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.add_custom_external_path("Original", "/my/path").await.unwrap();
        mgr.add_custom_external_path("Updated", "/my/path").await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].name, "Updated");
    }

    #[tokio::test]
    async fn remove_existing_path() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.add_custom_external_path("Skills", "/path/a").await.unwrap();
        mgr.add_custom_external_path("More", "/path/b").await.unwrap();

        mgr.remove_custom_external_path("/path/a").await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, "/path/b");
    }

    #[tokio::test]
    async fn remove_nonexistent_path_is_noop() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.remove_custom_external_path("/nonexistent").await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert!(paths.is_empty());
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn persists_and_reloads() {
        let tmp = TempDir::new().unwrap();

        // First session: add paths
        {
            let mgr = ExternalPathsManager::new(tmp.path()).await;
            mgr.add_custom_external_path("A", "/path/a").await.unwrap();
            mgr.add_custom_external_path("B", "/path/b").await.unwrap();
        }

        // Second session: paths should still be there
        {
            let mgr = ExternalPathsManager::new(tmp.path()).await;
            let paths = mgr.get_custom_external_paths().await;
            assert_eq!(paths.len(), 2);
            assert_eq!(paths[0].name, "A");
            assert_eq!(paths[1].name, "B");
        }
    }

    #[tokio::test]
    async fn handles_corrupted_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join(CUSTOM_SKILL_PATHS_FILE);
        std::fs::write(&file_path, "not valid json").unwrap();

        let mgr = ExternalPathsManager::new(tmp.path()).await;
        let paths = mgr.get_custom_external_paths().await;
        assert!(paths.is_empty());
    }

    // -----------------------------------------------------------------------
    // Skills market
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn enable_and_disable_skills_market() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.enable_skills_market().await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].name, SKILLS_MARKET_NAME);
        assert_eq!(paths[0].path, SKILLS_MARKET_PATH);

        mgr.disable_skills_market().await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert!(paths.is_empty());
    }

    #[tokio::test]
    async fn enable_market_idempotent() {
        let tmp = TempDir::new().unwrap();
        let mgr = ExternalPathsManager::new(tmp.path()).await;

        mgr.enable_skills_market().await.unwrap();
        mgr.enable_skills_market().await.unwrap();

        let paths = mgr.get_custom_external_paths().await;
        assert_eq!(paths.len(), 1);
    }
}
