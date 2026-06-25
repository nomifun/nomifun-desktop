/// Manifest filename that identifies an extension directory.
pub const EXTENSION_MANIFEST_FILE: &str = "nomi-extension.json";

/// Default subdirectory name for extensions.
pub const EXTENSIONS_DIR_NAME: &str = "extensions";

/// Current extension API version.
pub const EXTENSION_API_VERSION: &str = "1.0.0";

/// Hub index schema version we support.
pub const HUB_SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Cache TTL for agent activity snapshots (milliseconds).
pub const ACTIVITY_SNAPSHOT_TTL_MS: u64 = 3000;

/// Debounce delay for hot-reload file watching (milliseconds).
pub const DEBOUNCE_MS: u64 = 1000;

/// Debounce delay for state persistence writes (milliseconds).
pub const STATE_PERSIST_DEBOUNCE_MS: u64 = 500;

/// Reserved extension name prefixes that third-party extensions cannot use.
pub const RESERVED_NAME_PREFIXES: &[&str] = &["nomi-", "internal-", "builtin-", "system-"];

/// Preset agent type identifiers.
pub const PRESET_AGENT_TYPES: &[&str] = &["gemini", "claude", "codex", "codebuddy", "opencode"];

// ---------------------------------------------------------------------------
// Lifecycle hook timeouts (seconds)
// ---------------------------------------------------------------------------

/// Timeout for `onInstall` hook — may involve downloading dependencies.
pub const LIFECYCLE_ON_INSTALL_TIMEOUT_SECS: u64 = 120;

/// Timeout for `onUninstall` hook — cleanup operations.
pub const LIFECYCLE_ON_UNINSTALL_TIMEOUT_SECS: u64 = 60;

/// Timeout for `onActivate` hook — runs every activation.
pub const LIFECYCLE_ON_ACTIVATE_TIMEOUT_SECS: u64 = 30;

/// Timeout for `onDeactivate` hook — runs every deactivation.
pub const LIFECYCLE_ON_DEACTIVATE_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Reserved WebUI route prefixes
// ---------------------------------------------------------------------------

/// Route prefixes reserved for internal use — extensions cannot register these.
pub const RESERVED_ROUTE_PREFIXES: &[&str] = &["/api/", "/auth/", "/ws/"];

// ---------------------------------------------------------------------------
// Skill & rule management
// ---------------------------------------------------------------------------

/// Default subdirectory name for user-created skills.
pub const SKILLS_DIR_NAME: &str = "skills";

/// Default subdirectory name for per-job cron skills under the data dir.
pub const CRON_SKILLS_DIR_NAME: &str = "cron/skills";

/// Default subdirectory name for built-in skills.
pub const BUILTIN_SKILLS_DIR_NAME: &str = "builtin-skills";

/// Default subdirectory name for built-in rules.
pub const BUILTIN_RULES_DIR_NAME: &str = "builtin-rules";

/// Subdirectory inside the built-in skills corpus whose children are
/// auto-injected into every assistant. Historical name was `_builtin`;
/// renamed to `auto-inject` as part of the 2026-04-23 built-in skill
/// migration (skills are now embedded in the backend binary via
/// `include_dir!`).
pub const BUILTIN_AUTO_SKILLS_SUBDIR: &str = "auto-inject";

/// Default subdirectory name for assistant-level rules.
pub const ASSISTANT_RULES_DIR_NAME: &str = "assistant-rules";

/// Default subdirectory name for assistant-level skills.
pub const ASSISTANT_SKILLS_DIR_NAME: &str = "assistant-skills";

/// Filename that identifies a skill directory.
pub const SKILL_MANIFEST_FILE: &str = "SKILL.md";

/// Persistence file for custom external skill paths.
pub const CUSTOM_SKILL_PATHS_FILE: &str = "custom-skill-paths.json";

/// Well-known skill source name for the nomifun skills market.
pub const SKILLS_MARKET_NAME: &str = "nomifun-skills";

/// Well-known skill source path for the nomifun skills market.
///
/// NOTE: This is a URL placeholder, not a filesystem path. When used in
/// `ExternalPathsManager`, it serves as an identifier for the skills market
/// source. Filesystem scanning functions like `detect_and_count_external_skills`
/// will silently skip it since the path does not exist on disk.
pub const SKILLS_MARKET_PATH: &str = "https://github.com/nomifun/nomifun-skills";

/// Common skill directory names to detect on the filesystem.
///
/// Each tuple is `(display_name, relative_path, source_slug)`:
/// - `display_name` — user-facing label (e.g. the tab title).
/// - `relative_path` — path under the user's home directory.
/// - `source_slug` — stable machine-readable identifier mirrored to
///   the renderer as `ExternalSkillSourceResponse.source`. Used as a
///   React key and `data-testid` suffix in `SkillsHubSettings.tsx`.
pub const COMMON_SKILL_DIRS: &[(&str, &str, &str)] = &[
    ("Claude Skills", ".claude/skills", "claude"),
    ("Gemini Skills", ".gemini/skills", "gemini"),
    ("Codex / Agent Skills", ".agents/skills", "agents"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_file_name() {
        assert_eq!(EXTENSION_MANIFEST_FILE, "nomi-extension.json");
    }

    #[test]
    fn test_reserved_prefixes_contains_expected() {
        assert!(RESERVED_NAME_PREFIXES.contains(&"nomi-"));
        assert!(RESERVED_NAME_PREFIXES.contains(&"internal-"));
        assert!(RESERVED_NAME_PREFIXES.contains(&"builtin-"));
        assert!(RESERVED_NAME_PREFIXES.contains(&"system-"));
    }

    #[test]
    fn test_preset_agent_types_non_empty() {
        assert!(!PRESET_AGENT_TYPES.is_empty());
        assert!(PRESET_AGENT_TYPES.contains(&"claude"));
    }

    #[test]
    fn test_lifecycle_timeouts_ordering() {
        // onInstall should have the longest timeout
        const {
            assert!(LIFECYCLE_ON_INSTALL_TIMEOUT_SECS >= LIFECYCLE_ON_ACTIVATE_TIMEOUT_SECS);
            assert!(LIFECYCLE_ON_INSTALL_TIMEOUT_SECS >= LIFECYCLE_ON_DEACTIVATE_TIMEOUT_SECS);
            assert!(LIFECYCLE_ON_UNINSTALL_TIMEOUT_SECS >= LIFECYCLE_ON_DEACTIVATE_TIMEOUT_SECS);
        }
    }

    #[test]
    fn test_reserved_route_prefixes() {
        assert!(RESERVED_ROUTE_PREFIXES.contains(&"/api/"));
        assert!(RESERVED_ROUTE_PREFIXES.contains(&"/auth/"));
        assert!(RESERVED_ROUTE_PREFIXES.contains(&"/ws/"));
    }

    #[test]
    fn test_debounce_values_positive() {
        const {
            assert!(DEBOUNCE_MS > 0);
            assert!(STATE_PERSIST_DEBOUNCE_MS > 0);
            assert!(ACTIVITY_SNAPSHOT_TTL_MS > 0);
        }
    }

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
