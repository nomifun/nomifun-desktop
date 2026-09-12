use std::collections::HashSet;
use std::path::{Path, PathBuf};

use futures::future::join_all;

use crate::frontmatter::{parse_frontmatter, parse_skill_fields};
use crate::mcp::load_mcp_skills;
use crate::paths::{
    additional_skills_dirs, project_commands_dirs, project_skills_dirs, user_skills_dir,
};
use crate::types::{LoadedFrom, SkillMetadata, SkillSource};
use nomi_mcp::manager::McpManager;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A loaded skill paired with its canonical filesystem path for deduplication.
pub struct LoadedSkill {
    pub metadata: SkillMetadata,
    /// Canonicalized path used for dedup (symlinks resolved, `.`/`..` removed).
    pub resolved_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load all skills from the filesystem and optionally from MCP servers.
///
/// Priority order (highest first): MCP → user → project → additional → legacy.
/// Deduplicates first by canonical path (symlinks resolved), then by name (first wins).
///
/// If `bare` is true, only `add_dirs` are consulted (used for isolated
/// environments where the user/project directories should be ignored).
///
/// Pass `mcp_manager: Some(&manager)` to include MCP-discovered skills.
pub async fn load_all_skills(
    cwd: &Path,
    add_dirs: &[PathBuf],
    bare: bool,
    mcp_manager: Option<&McpManager>,
) -> Vec<SkillMetadata> {
    let mut all: Vec<LoadedSkill> = Vec::new();

    if bare {
        // Bare mode: only load from explicit add_dirs
        let dirs = additional_skills_dirs(add_dirs);
        let futures: Vec<_> = dirs
            .iter()
            .map(|d| load_skills_from_dir(d, SkillSource::Project, LoadedFrom::Skills))
            .collect();
        for batch in join_all(futures).await {
            all.extend(batch);
        }
        return deduplicate_by_name(deduplicate(all));
    }

    // 1. User-level skills (highest priority)
    if let Some(dir) = user_skills_dir()
        && dir.is_dir()
    {
        all.extend(load_skills_from_dir(&dir, SkillSource::User, LoadedFrom::Skills).await);
    }

    // 2. Project-level skills (parallel across all dirs)
    let project_dirs = project_skills_dirs(cwd);
    let futures: Vec<_> = project_dirs
        .iter()
        .map(|d| load_skills_from_dir(d, SkillSource::Project, LoadedFrom::Skills))
        .collect();
    for batch in join_all(futures).await {
        all.extend(batch);
    }

    // 3. Additional dirs from --add-dir
    let add_skill_dirs = additional_skills_dirs(add_dirs);
    let futures: Vec<_> = add_skill_dirs
        .iter()
        .map(|d| load_skills_from_dir(d, SkillSource::Project, LoadedFrom::Skills))
        .collect();
    for batch in join_all(futures).await {
        all.extend(batch);
    }

    // 4. Project-level legacy commands (parallel). The v3 product dataset no
    // longer reads a second user-global commands root from platform config;
    // project-owned `.nomi/commands` remains an explicit workspace input.
    let cmd_dirs = project_commands_dirs(cwd);
    let futures: Vec<_> = cmd_dirs
        .iter()
        .map(|d| load_skills_from_commands_dir(d, SkillSource::Project))
        .collect();
    for batch in join_all(futures).await {
        all.extend(batch);
    }

    // MCP skills inserted before filesystem skills, so:
    // MCP > user > project > additional > legacy.
    let mcp_loaded = match mcp_manager {
        Some(mgr) => load_mcp_skills(mgr).await,
        None => Vec::new(),
    };

    all.splice(0..0, mcp_loaded);

    // Path-based dedup first (handles symlinked duplicates), then name-based
    // dedup to enforce MCP vs. filesystem priority.
    deduplicate_by_name(deduplicate(all))
}

// ---------------------------------------------------------------------------
// Internal: load from skills/ directory (directory-only format)
// ---------------------------------------------------------------------------

/// Load skills from a `skills/` directory.
///
/// Only the directory format is supported: each direct or nested subdirectory
/// that contains a `SKILL.md` file (case-sensitive) is loaded.
/// The skill name is derived from the relative path using colon separators.
pub(crate) async fn load_skills_from_dir(
    base_dir: &Path,
    source: SkillSource,
    loaded_from: LoadedFrom,
) -> Vec<LoadedSkill> {
    let mut results = Vec::new();
    collect_skills(
        base_dir,
        base_dir,
        source,
        loaded_from,
        &mut HashSet::new(),
        &mut results,
    )
    .await;
    results
}

/// Load legacy commands: directory skills take precedence over flat Markdown.
async fn load_skills_from_commands_dir(base_dir: &Path, source: SkillSource) -> Vec<LoadedSkill> {
    load_skills_from_dir(base_dir, source, LoadedFrom::CommandsDeprecated).await
}

/// Share traversal and cycle detection between skills and legacy commands.
fn collect_skills<'a>(
    base_dir: &'a Path,
    dir: &'a Path,
    source: SkillSource,
    loaded_from: LoadedFrom,
    ancestors: &'a mut HashSet<PathBuf>,
    results: &'a mut Vec<LoadedSkill>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let mut read_dir = match tokio::fs::read_dir(dir).await {
            Ok(rd) => rd,
            Err(_) => return,
        };
        let Ok(canonical) = tokio::fs::canonicalize(dir).await else {
            return;
        };
        if !ancestors.insert(canonical.clone()) {
            return;
        }

        let mut entries = Vec::new();
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            // Resolve links once, preserving linked skills and namespace directories.
            if let Ok(metadata) = tokio::fs::metadata(&path).await {
                entries.push((path, metadata));
            }
        }
        // First-wins dedup must not depend on filesystem enumeration order.
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut dir_names = HashSet::new();
        for (path, metadata) in &entries {
            if !metadata.is_dir() {
                continue;
            }
            if let Some(skill_file) = find_exact_file(path, "SKILL.md").await {
                if let Some(skill) =
                    load_skill_file(&skill_file, base_dir, path, source, loaded_from).await
                {
                    dir_names.insert(path.file_name().unwrap_or_default());
                    results.push(skill);
                }
            } else {
                collect_skills(base_dir, path, source, loaded_from, ancestors, results).await;
            }
        }

        if loaded_from == LoadedFrom::CommandsDeprecated {
            for (path, metadata) in &entries {
                if !metadata.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let stem = path.file_stem().unwrap_or_default();
                if dir_names.contains(stem) {
                    continue;
                }
                let pseudo_dir = dir.join(stem);
                if let Some(skill) =
                    load_skill_file(path, base_dir, &pseudo_dir, source, loaded_from).await
                {
                    results.push(skill);
                }
            }
        }
        ancestors.remove(&canonical);
    })
}

// ---------------------------------------------------------------------------
// Internal: load a single skill file
// ---------------------------------------------------------------------------

/// Read, parse, and return a `LoadedSkill` for a single Markdown file.
/// Returns `None` if the file cannot be read.
async fn load_skill_file(
    file_path: &Path,
    base_dir: &Path,
    skill_dir: &Path,
    source: SkillSource,
    loaded_from: LoadedFrom,
) -> Option<LoadedSkill> {
    let content = tokio::fs::read_to_string(file_path).await.ok()?;
    let parsed = parse_frontmatter(&content);

    let resolved_name = build_namespace(base_dir, skill_dir);
    // Flat legacy commands have no physical directory named after their stem.
    let skill_root = file_path.parent().map(|dir| dir.to_string_lossy().into_owned());

    let metadata = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        &resolved_name,
        source,
        loaded_from,
        skill_root.as_deref(),
    );

    let resolved_path = try_canonicalize(file_path).unwrap_or_else(|| file_path.to_owned());

    Some(LoadedSkill {
        metadata,
        resolved_path,
    })
}

// ---------------------------------------------------------------------------
// Internal: namespace building
// ---------------------------------------------------------------------------

/// Build a colon-separated namespace from a directory hierarchy.
///
/// Examples:
/// - base=`<config_dir>/nomi/skills`, target=`<config_dir>/nomi/skills/db/migrate` → `"db:migrate"`
/// - base=`<config_dir>/nomi/skills`, target=`<config_dir>/nomi/skills/my-skill` → `"my-skill"`
pub(crate) fn build_namespace(base_dir: &Path, target_dir: &Path) -> String {
    match target_dir.strip_prefix(base_dir) {
        Ok(relative) => relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":"),
        Err(_) => target_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Internal: deduplication
// ---------------------------------------------------------------------------

/// Deduplicate loaded skills by canonical path. First occurrence wins.
fn deduplicate(skills: Vec<LoadedSkill>) -> Vec<SkillMetadata> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut result = Vec::new();

    for skill in skills {
        if seen.insert(skill.resolved_path) {
            result.push(skill.metadata);
        }
    }

    result
}

/// Deduplicate by skill name (case-sensitive). First occurrence wins.
///
/// Called after path-based dedup to enforce priority between bundled, MCP,
/// and filesystem skills that share the same name but have different paths.
fn deduplicate_by_name(skills: Vec<SkillMetadata>) -> Vec<SkillMetadata> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for skill in skills {
        if seen.insert(skill.name.clone()) {
            result.push(skill);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Internal: safe canonicalize
// ---------------------------------------------------------------------------

/// Canonicalize a path, returning `None` if the path does not exist.
/// Never panics.
pub(crate) fn try_canonicalize(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Find a file with an exact case-sensitive name inside `dir`.
///
/// On case-insensitive filesystems (e.g., macOS APFS), `Path::is_file()` may
/// return `true` for `SKILL.md` even when only `skill.md` exists.  This
/// function reads the directory entries and performs a byte-for-byte name
/// comparison to avoid false positives.
///
/// Returns `None` if no entry with that exact name exists or if the directory
/// cannot be read.
async fn find_exact_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut rd = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = rd.next_entry().await {
        if entry.file_name().to_string_lossy() == name {
            let path = entry.path();
            let ft = entry.file_type().await.ok()?;
            if ft.is_file() {
                return Some(path);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "loader_supplemental_tests.rs"]
mod supplemental_tests;
