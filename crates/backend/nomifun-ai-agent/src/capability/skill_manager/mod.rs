use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};

mod prompt_builder;
pub use prompt_builder::*;

/// A discovered skill definition.
#[derive(Debug, Clone)]
pub struct SkillDefinition {
    /// Skill name (directory name or frontmatter `name`).
    pub name: String,
    /// One-line description from SKILL.md frontmatter.
    pub description: String,
    /// File system path to the SKILL.md file (absolute for custom/extension,
    /// or the materialized view path for builtin).
    pub location: PathBuf,
    /// Origin of this skill (builtin/custom/extension).
    pub source: nomifun_extension::SkillSource,
    /// Relative path inside the builtin skill corpus
    /// (e.g. `auto-inject/cron/SKILL.md`); `None` for non-builtin sources.
    pub relative_location: Option<String>,
}

/// Lightweight skill reference for index listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndex {
    pub name: String,
    pub description: String,
}

/// Manages skill discovery and indexing for first-message injection.
///
/// Skills are stored in directories containing a `SKILL.md` file.
/// The SKILL.md frontmatter provides `name` and `description`.
pub struct AcpSkillManager {
    /// Cached skill definitions keyed by skill name.
    cache: RwLock<HashMap<String, SkillDefinition>>,
    /// Resolved skill paths, shared across the app.
    paths: Arc<nomifun_extension::SkillPaths>,
}

impl AcpSkillManager {
    pub fn new(paths: Arc<nomifun_extension::SkillPaths>) -> Arc<Self> {
        Arc::new(Self {
            cache: RwLock::new(HashMap::new()),
            paths,
        })
    }

    /// Populate the cache with only the named skills (no filtering by
    /// auto-inject/opt-in). Returns the resulting index. Used by the
    /// snapshot-driven first-message injector.
    pub async fn discover_by_names(&self, names: &[String]) -> Vec<SkillIndex> {
        // Always reset state so repeated calls produce a deterministic cache.
        if names.is_empty() {
            let mut cache = self.cache.write().await;
            cache.clear();
            return Vec::new();
        }
        let items = match nomifun_extension::list_available_skills(&self.paths).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "discover_by_names: list_available_skills failed");
                Vec::new()
            }
        };

        let wanted: std::collections::HashSet<&String> = names.iter().collect();
        let mut cache = self.cache.write().await;
        cache.clear();
        for item in items {
            if !wanted.contains(&item.name) {
                continue;
            }
            cache.insert(
                item.name.clone(),
                SkillDefinition {
                    name: item.name.clone(),
                    description: item.description.clone(),
                    location: std::path::PathBuf::from(&item.location),
                    source: item.source,
                    relative_location: item.relative_location.clone(),
                },
            );
        }
        let index: Vec<SkillIndex> = cache
            .values()
            .map(|d| SkillIndex {
                name: d.name.clone(),
                description: d.description.clone(),
            })
            .collect();
        debug!(count = index.len(), "Skills discovered by name");
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn new_accepts_skill_paths() {
        let tmp = TempDir::new().unwrap();
        let paths = std::sync::Arc::new(nomifun_extension::resolve_skill_paths(tmp.path(), tmp.path()));
        let mgr = AcpSkillManager::new(paths.clone());
        assert!(mgr.discover_by_names(&[]).await.is_empty());
    }

    #[test]
    fn skill_definition_has_source_and_relative_location() {
        let def = SkillDefinition {
            name: "x".into(),
            description: "d".into(),
            location: PathBuf::from("/tmp/x"),
            source: nomifun_extension::SkillSource::Builtin,
            relative_location: Some("auto-inject/x/SKILL.md".into()),
        };
        assert_eq!(def.source, nomifun_extension::SkillSource::Builtin);
        assert_eq!(def.relative_location.as_deref(), Some("auto-inject/x/SKILL.md"));
    }

    // Frontmatter parsing tests live in nomifun-extension (covers
    // parse_frontmatter_fields there); removed from here when
    // skill_manager stopped owning that helper.
}
