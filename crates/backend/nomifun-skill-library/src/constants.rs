/// Default subdirectory name for user-created skills.
pub const SKILLS_DIR_NAME: &str = "skills";

/// Default subdirectory name for per-job cron skills under the data dir.
pub const CRON_SKILLS_DIR_NAME: &str = "cron/skills";

/// Default subdirectory name for built-in skills.
pub const BUILTIN_SKILLS_DIR_NAME: &str = "builtin-skills";

/// Default subdirectory name for built-in rules.
pub const BUILTIN_RULES_DIR_NAME: &str = "builtin-rules";

/// Subdirectory inside the built-in skills corpus whose children are
/// auto-injected into every preset. Historical name was `_builtin`;
/// renamed to `auto-inject` as part of the 2026-04-23 built-in skill
/// migration.
pub const BUILTIN_AUTO_SKILLS_SUBDIR: &str = "auto-inject";

/// Filename that identifies a skill directory.
pub const SKILL_MANIFEST_FILE: &str = "SKILL.md";

/// Persistence file for custom external skill paths.
pub const CUSTOM_SKILL_PATHS_FILE: &str = "custom-skill-paths.json";

/// Well-known skill source name for the nomifun skills market.
pub const SKILLS_MARKET_NAME: &str = "nomifun-skills";

/// Well-known skill source path for the nomifun skills market.
///
/// This URL is an external-source identifier, not a filesystem path.
/// Filesystem scanners skip it because it does not exist on disk.
pub const SKILLS_MARKET_PATH: &str = "https://github.com/nomifun/nomifun-skills";

/// Common skill directory names to detect on the filesystem.
///
/// Each tuple is `(display_name, relative_path, source_slug)`:
/// - `display_name` is user-facing.
/// - `relative_path` is resolved under the user's home directory.
/// - `source_slug` is the stable renderer-facing identifier.
pub const COMMON_SKILL_DIRS: &[(&str, &str, &str)] = &[
    ("Claude Skills", ".claude/skills", "claude"),
    ("Gemini Skills", ".gemini/skills", "gemini"),
    ("Codex / Agent Skills", ".agents/skills", "agents"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_skill_dirs_include_codex_agent_skills_home() {
        let codex = COMMON_SKILL_DIRS
            .iter()
            .find(|(_, _, slug)| *slug == "agents")
            .expect("common Agent Skills source must exist");

        assert_eq!(
            *codex,
            ("Codex / Agent Skills", ".agents/skills", "agents"),
            "Codex reads user skills from ~/.agents/skills, not the broader ~/.agents folder"
        );
    }
}
