use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, warn};

mod prompt_builder;
pub use prompt_builder::*;

static LOAD_SKILL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[LOAD_SKILL:\s*([^\]]+)\]").expect("valid regex"));

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
    /// Lazily-loaded full content (body after frontmatter).
    pub body: Option<String>,
}

/// Lightweight skill reference for index listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndex {
    pub name: String,
    pub description: String,
}

/// Manages skill discovery, indexing, and on-demand loading.
///
/// Skills are stored in directories containing a `SKILL.md` file.
/// The SKILL.md frontmatter provides `name` and `description`.
/// The body (content after frontmatter) is loaded on demand.
pub struct AcpSkillManager {
    /// Cached skill definitions keyed by skill name.
    cache: RwLock<HashMap<String, SkillDefinition>>,
    /// Whether discovery has been performed.
    discovered: RwLock<bool>,
    /// Resolved skill paths, shared across the app.
    /// Consumed by `discover_skills` / `get_skill` (Task 4 / 5 of the refactor).
    #[allow(dead_code)]
    paths: Arc<nomifun_extension::SkillPaths>,
}

impl AcpSkillManager {
    pub fn new(paths: Arc<nomifun_extension::SkillPaths>) -> Arc<Self> {
        Arc::new(Self {
            cache: RwLock::new(HashMap::new()),
            discovered: RwLock::new(false),
            paths,
        })
    }

    /// Discover skills via `nomifun_extension::list_available_skills`.
    ///
    /// Filtering rules:
    /// - Auto-inject builtin skills (under `auto-inject/` in the corpus) are
    ///   always included unless listed in `exclude_builtin_skills`.
    /// - Opt-in builtin skills (siblings of `auto-inject/`) and custom/extension
    ///   skills are included only if `enabled_skills` contains their name.
    ///
    /// Populates the cache; subsequent `get_skill(name)` calls read body lazily.
    pub async fn discover_skills(
        &self,
        enabled_skills: Option<&[String]>,
        exclude_builtin_skills: Option<&[String]>,
    ) -> Vec<SkillIndex> {
        let items = match nomifun_extension::list_available_skills(&self.paths).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "Failed to list skills via extension service");
                Vec::new()
            }
        };

        let mut cache = self.cache.write().await;
        cache.clear();

        for item in items {
            let is_auto_inject = item
                .relative_location
                .as_deref()
                .is_some_and(|r| r.starts_with("auto-inject/"));

            let keep = match item.source {
                nomifun_extension::SkillSource::Builtin => {
                    if is_auto_inject {
                        !exclude_builtin_skills.is_some_and(|ex| ex.iter().any(|n| n == &item.name))
                    } else {
                        enabled_skills.is_some_and(|en| en.iter().any(|n| n == &item.name))
                    }
                }
                nomifun_extension::SkillSource::Custom | nomifun_extension::SkillSource::Extension => {
                    enabled_skills.is_some_and(|en| en.iter().any(|n| n == &item.name))
                }
            };
            if !keep {
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
                    body: None,
                },
            );
        }

        let mut discovered = self.discovered.write().await;
        *discovered = true;

        let index: Vec<SkillIndex> = cache
            .values()
            .map(|d| SkillIndex {
                name: d.name.clone(),
                description: d.description.clone(),
            })
            .collect();

        debug!(count = index.len(), "Skills discovered");
        index
    }

    /// Populate the cache with only the named skills (no filtering by
    /// auto-inject/opt-in). Returns the resulting index. Used by the
    /// snapshot-driven first-message injector.
    pub async fn discover_by_names(&self, names: &[String]) -> Vec<SkillIndex> {
        // Always reset state so repeated calls produce a deterministic cache.
        if names.is_empty() {
            let mut cache = self.cache.write().await;
            cache.clear();
            let mut discovered = self.discovered.write().await;
            *discovered = true;
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
                    body: None,
                },
            );
        }
        let mut discovered = self.discovered.write().await;
        *discovered = true;
        cache
            .values()
            .map(|d| SkillIndex {
                name: d.name.clone(),
                description: d.description.clone(),
            })
            .collect()
    }

    /// Load a skill's full content by name.
    ///
    /// Returns `None` if the skill is unknown. On first access the body is
    /// read via the appropriate channel based on `source`:
    /// - `Builtin` → `nomifun_extension::read_builtin_skill(&paths, relative)`
    /// - `Custom` / `Extension` → direct `tokio::fs::read_to_string(location/SKILL.md)`
    pub async fn get_skill(&self, name: &str) -> Option<SkillDefinition> {
        // Fast path: check if body is already cached
        {
            let cache = self.cache.read().await;
            match cache.get(name) {
                Some(def) if def.body.is_some() => return Some(def.clone()),
                None => return None,
                _ => {} // known, body absent — fall through
            }
        }

        // Slow path: read body per source and cache it
        let mut cache = self.cache.write().await;
        let def = cache.get_mut(name)?;
        if def.body.is_some() {
            return Some(def.clone());
        }

        let content = match def.source {
            nomifun_extension::SkillSource::Builtin => {
                if let Some(rel) = def.relative_location.as_deref() {
                    match nomifun_extension::read_builtin_skill(&self.paths, rel).await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(skill = name, error = %e, "Failed to read builtin skill");
                            String::new()
                        }
                    }
                } else {
                    warn!(skill = name, "Builtin skill missing relative_location");
                    String::new()
                }
            }
            nomifun_extension::SkillSource::Custom | nomifun_extension::SkillSource::Extension => {
                // `location` for scanned user skills is the directory; append SKILL.md.
                let skill_file = if def.location.is_dir() {
                    def.location.join("SKILL.md")
                } else {
                    def.location.clone()
                };
                match tokio::fs::read_to_string(&skill_file).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(skill = name, path = %skill_file.display(), error = %e, "Failed to read skill file");
                        String::new()
                    }
                }
            }
        };

        if content.is_empty() {
            return None;
        }

        def.body = Some(extract_body(&content));
        Some(def.clone())
    }

    /// Check whether discovery has been performed.
    pub async fn is_discovered(&self) -> bool {
        *self.discovered.read().await
    }
}

/// Detect `[LOAD_SKILL: ...]` requests in agent output content.
///
/// Returns a list of requested skill names.
pub fn detect_skill_load_request(content: &str) -> Vec<String> {
    LOAD_SKILL_RE
        .captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|name| !name.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the body content after YAML frontmatter.
fn extract_body(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    let after_open = &trimmed[3..];
    if let Some(close_idx) = after_open.find("---") {
        let after_close = &after_open[close_idx + 3..];
        after_close.trim_start_matches('\n').to_string()
    } else {
        content.to_string()
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
        assert!(!mgr.is_discovered().await);
    }

    #[test]
    fn skill_definition_has_source_and_relative_location() {
        let def = SkillDefinition {
            name: "x".into(),
            description: "d".into(),
            location: PathBuf::from("/tmp/x"),
            source: nomifun_extension::SkillSource::Builtin,
            relative_location: Some("auto-inject/x/SKILL.md".into()),
            body: None,
        };
        assert_eq!(def.source, nomifun_extension::SkillSource::Builtin);
        assert_eq!(def.relative_location.as_deref(), Some("auto-inject/x/SKILL.md"));
    }

    // Frontmatter parsing tests live in nomifun-extension (covers
    // parse_frontmatter_fields there); removed from here when
    // skill_manager stopped owning that helper.

    // -----------------------------------------------------------------------
    // Body extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_body_with_frontmatter() {
        let content = "---\nname: test\ndescription: desc\n---\nBody content\nMore lines";
        let body = extract_body(content);
        assert_eq!(body, "Body content\nMore lines");
    }

    #[test]
    fn extract_body_no_frontmatter() {
        let content = "Just plain text";
        assert_eq!(extract_body(content), "Just plain text");
    }

    #[test]
    fn extract_body_no_closing_delimiter() {
        let content = "---\nname: test\nno closing";
        assert_eq!(extract_body(content), content);
    }

    // -----------------------------------------------------------------------
    // LOAD_SKILL detection
    // -----------------------------------------------------------------------

    #[test]
    fn detect_single_skill_request() {
        let content = "Let me use [LOAD_SKILL: security-review] for this.";
        let skills = detect_skill_load_request(content);
        assert_eq!(skills, vec!["security-review"]);
    }

    #[test]
    fn detect_multiple_skill_requests() {
        let content = "[LOAD_SKILL: a] some text [LOAD_SKILL: b]";
        let skills = detect_skill_load_request(content);
        assert_eq!(skills, vec!["a", "b"]);
    }

    #[test]
    fn detect_skill_request_with_spaces() {
        let content = "[LOAD_SKILL:   padded-name   ]";
        let skills = detect_skill_load_request(content);
        assert_eq!(skills, vec!["padded-name"]);
    }

    #[test]
    fn detect_no_skill_request() {
        let content = "Just regular text with no commands.";
        let skills = detect_skill_load_request(content);
        assert!(skills.is_empty());
    }

    #[test]
    fn detect_skill_request_empty_name_ignored() {
        let content = "[LOAD_SKILL:  ]";
        let skills = detect_skill_load_request(content);
        assert!(skills.is_empty());
    }

    // -----------------------------------------------------------------------
    // AcpSkillManager async tests
    //
    // Discovery-layout tests moved to `tests/skill_manager_integration.rs`
    // because they now need `nomifun_extension::BUILTIN_SKILLS_ENV_VAR` to
    // point the extension service at a tempdir corpus. Only the tests that
    // don't require a skill corpus remain here.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_skill_unknown_returns_none() {
        let tmp = TempDir::new().unwrap();
        let mgr = AcpSkillManager::new(std::sync::Arc::new(nomifun_extension::resolve_skill_paths(
            tmp.path(),
            tmp.path(),
        )));
        assert!(mgr.get_skill("nonexistent").await.is_none());
    }
}
