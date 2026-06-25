//! Shared first-message prefix injection for ACP agents.
//!
//! Takes the conversation's first-message content and produces a new content
//! string that may include an `[Assistant Rules]` block with preset context
//! and a skills index. The shape depends on whether the agent's native CLI
//! can read skills from the workspace directly.

use std::sync::Arc;

use crate::capability::skill_manager::{AcpSkillManager, prepare_first_message_with_skills_index};

/// Configuration for the first-message injector.
pub struct InjectionConfig<'a> {
    /// Preset context (assistant-level system prompt injection).
    pub preset_context: Option<&'a str>,
    /// Resolved skill names (snapshot from `conversation.extra.skills`).
    pub skills: &'a [String],
    /// True iff the agent's native CLI reads skills from the workspace
    /// without needing prompt injection. Derived by callers from
    /// `AcpBackend::native_skills_dirs().is_some()` for ACP, or hardcoded
    /// `false` for nomi / custom workspace scenarios.
    pub native_skill_support: bool,
    /// True iff the user chose a custom workspace (symlinks may not exist).
    pub custom_workspace: bool,
}

/// Produce the content string to send as the first ACP prompt.
///
/// - If `native_skill_support && !custom_workspace`: **light mode** — only
///   `preset_context` prepended as an `[Assistant Rules]` block (if present).
///   The native CLI handles skill discovery via workspace symlinks.
/// - Else: **heavy mode** — `preset_context` + resolved skills index
///   injected via `prepare_first_message_with_skills_index`.
pub async fn inject_first_message_prefix(
    content: &str,
    manager: &Arc<AcpSkillManager>,
    config: InjectionConfig<'_>,
) -> String {
    let use_native = config.native_skill_support && !config.custom_workspace;

    if use_native {
        return match config.preset_context {
            Some(ctx) if !ctx.is_empty() => {
                format!("[Assistant Rules]\n{ctx}\n[/Assistant Rules]\n\n{content}")
            }
            _ => content.to_string(),
        };
    }

    let skills = manager.discover_by_names(config.skills).await;
    let has_context = config.preset_context.is_some_and(|s| !s.is_empty());
    if skills.is_empty() && !has_context {
        return content.to_string();
    }
    prepare_first_message_with_skills_index(content, &skills, config.preset_context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_extension::{BUILTIN_SKILLS_ENV_VAR, resolve_skill_paths};
    use tempfile::TempDir;

    fn test_mgr(base: &std::path::Path) -> Arc<AcpSkillManager> {
        let paths = Arc::new(resolve_skill_paths(base, base));
        AcpSkillManager::new(paths)
    }

    /// Point the embedded corpus at an empty dir so tests don't pick up
    /// real auto-inject builtin skills.
    struct EmptyBuiltinGuard;
    impl EmptyBuiltinGuard {
        fn new(empty_path: &std::path::Path) -> Self {
            unsafe {
                std::env::set_var(BUILTIN_SKILLS_ENV_VAR, empty_path);
            }
            Self
        }
    }
    impl Drop for EmptyBuiltinGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var(BUILTIN_SKILLS_ENV_VAR);
            }
        }
    }

    #[tokio::test]
    async fn light_mode_with_preset_context() {
        let tmp = TempDir::new().unwrap();
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Hello",
            &mgr,
            InjectionConfig {
                preset_context: Some("Be concise."),
                skills: &[],
                native_skill_support: true,
                custom_workspace: false,
            },
        )
        .await;

        assert!(out.contains("[Assistant Rules]"));
        assert!(out.contains("Be concise."));
        assert!(out.ends_with("Hello"));
    }

    #[tokio::test]
    async fn light_mode_empty_context_passes_through() {
        let tmp = TempDir::new().unwrap();
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Hello",
            &mgr,
            InjectionConfig {
                preset_context: None,
                skills: &[],
                native_skill_support: true,
                custom_workspace: false,
            },
        )
        .await;
        assert_eq!(out, "Hello");
    }

    #[tokio::test]
    async fn heavy_mode_no_skills_no_context_passes_through() {
        let tmp = TempDir::new().unwrap();
        let _guard = EmptyBuiltinGuard::new(tmp.path());
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Hello",
            &mgr,
            InjectionConfig {
                preset_context: None,
                skills: &[],
                native_skill_support: false,
                custom_workspace: false,
            },
        )
        .await;
        assert_eq!(out, "Hello");
    }

    #[tokio::test]
    async fn heavy_mode_with_preset_context_no_skills() {
        let tmp = TempDir::new().unwrap();
        let _guard = EmptyBuiltinGuard::new(tmp.path());
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Go.",
            &mgr,
            InjectionConfig {
                preset_context: Some("Rule 1."),
                skills: &[],
                native_skill_support: false,
                custom_workspace: false,
            },
        )
        .await;

        assert!(out.contains("[Assistant Rules]"));
        assert!(out.contains("Rule 1."));
        assert!(out.ends_with("Go."));
    }

    #[tokio::test]
    async fn heavy_mode_with_resolved_skills_injects_index() {
        // Set up a builtin skills dir with two skills; pass only one in `skills`.
        let tmp = TempDir::new().unwrap();
        let auto = tmp.path().join("auto-inject");
        std::fs::create_dir_all(auto.join("cron")).unwrap();
        std::fs::write(
            auto.join("cron").join("SKILL.md"),
            "---\nname: cron\ndescription: Schedule stuff\n---\nBody.",
        )
        .unwrap();
        std::fs::create_dir_all(auto.join("pdf")).unwrap();
        std::fs::write(
            auto.join("pdf").join("SKILL.md"),
            "---\nname: pdf\ndescription: Render PDFs\n---\nBody.",
        )
        .unwrap();
        let _guard = EmptyBuiltinGuard::new(tmp.path());
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Hello",
            &mgr,
            InjectionConfig {
                preset_context: None,
                skills: &["cron".to_owned()],
                native_skill_support: false,
                custom_workspace: false,
            },
        )
        .await;
        assert!(out.contains("cron"));
        assert!(!out.contains("pdf"));
        assert!(out.ends_with("Hello"));
    }

    #[tokio::test]
    async fn custom_workspace_forces_heavy_even_when_native_supported() {
        let tmp = TempDir::new().unwrap();
        let _guard = EmptyBuiltinGuard::new(tmp.path());
        let mgr = test_mgr(tmp.path());

        let out = inject_first_message_prefix(
            "Do stuff",
            &mgr,
            InjectionConfig {
                preset_context: Some("Custom rule"),
                skills: &[],
                native_skill_support: true,
                custom_workspace: true, // <-- overrides native
            },
        )
        .await;

        assert!(out.contains("[Assistant Rules]"));
        assert!(out.contains("Custom rule"));
    }
}
