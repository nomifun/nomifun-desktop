pub mod bash;
pub mod apply_patch;
pub mod edit;
pub mod exec_command;
pub mod file_cache;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod output_truncation;
pub mod path_guard;
pub mod persistent_shell;
pub mod process_store;
pub mod pty;
pub mod read;
pub mod registry;
pub mod sandbox;
pub mod tool_search;
pub mod update_plan;
pub mod write;
pub mod write_stdin;
pub mod worktree;

/// Shared test-only helpers (path to the cross-platform `pty_test_helper` bin).
#[cfg(test)]
pub(crate) mod test_support;


pub use output_truncation::{approx_token_count, truncate_middle, TruncationBudget};

use async_trait::async_trait;
use serde_json::Value;

use nomi_config::hooks::HooksConfig;
use nomi_protocol::events::ToolCategory;
use nomi_types::skill_types::ContextModifier;
use nomi_types::tool::{JsonSchema, ToolResult};

/// Truncate a string to at most `max_bytes`, snapping to a char boundary.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Write `content` to `file_path` atomically: write to a uniquely-named temp
/// file in the same directory, then rename it over the target. Rename is atomic
/// on the same filesystem, so a crash or a concurrent reader never observes a
/// half-written file. Falls back to a direct write only if the rename fails
/// (e.g. cross-device). Shared by the Edit and Write tools so both get the same
/// crash-safety guarantee.
pub(crate) fn atomic_write(file_path: &str, content: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = format!("{}.tmp.{}.{}", file_path, std::process::id(), seq);

    if let Err(e) = std::fs::write(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    if std::fs::rename(&tmp_path, file_path).is_err() {
        // Cross-device rename (temp and target on different filesystems) cannot
        // be atomic; clean up the temp and fall back to a direct write.
        let _ = std::fs::remove_file(&tmp_path);
        std::fs::write(file_path, content)?;
    }
    Ok(())
}

/// A tool that the agent can invoke
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (must match API schema)
    fn name(&self) -> &str;

    /// Human-readable description for the LLM
    fn description(&self) -> &str;

    /// JSON Schema for input parameters
    fn input_schema(&self) -> JsonSchema;

    /// Whether this tool is safe to run concurrently
    fn is_concurrency_safe(&self, input: &Value) -> bool;

    /// Execute the tool
    async fn execute(&self, input: Value) -> ToolResult;

    /// Return an optional context modifier based on the tool input.
    /// Called after execute() to collect any engine-level overrides.
    /// Only SkillTool overrides this; all other tools return None.
    fn context_modifier_for(&self, _input: &Value) -> Option<ContextModifier> {
        None
    }

    /// Return any hooks declared in the skill's frontmatter for dynamic registration.
    /// Called after a successful execute() so the orchestration layer can merge
    /// the returned hooks into the active HookEngine.
    /// Only SkillTool overrides this; all other tools return None.
    fn skill_hooks_for(&self, _input: &Value) -> Option<HooksConfig> {
        None
    }

    /// Max result size in chars before truncation
    fn max_result_size(&self) -> usize {
        50_000
    }

    /// Tool category for protocol classification
    fn category(&self) -> ToolCategory;

    /// Category for a specific invocation. Lets multi-action tools (e.g.
    /// Computer/Browser) report read-only actions as Info so approval
    /// gating can distinguish them from mutating actions.
    fn category_for(&self, _input: &Value) -> ToolCategory {
        self.category()
    }

    /// Whether this tool's schema should be deferred (sent as name-only stub).
    /// Override to `true` for tools with large schemas or infrequent use.
    fn is_deferred(&self) -> bool {
        false
    }

    /// Human-readable description of what the tool will do with the given input
    fn describe(&self, input: &Value) -> String {
        format!(
            "{}: {}",
            self.name(),
            serde_json::to_string(input).unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_ascii_within_limit() {
        assert_eq!(truncate_utf8("hello", 80), "hello");
    }

    #[test]
    fn truncate_utf8_ascii_at_boundary() {
        assert_eq!(truncate_utf8("abcde", 3), "abc");
    }

    #[test]
    fn truncate_utf8_multibyte_snaps_back() {
        // '些' is 3 bytes (E4 BA 9B) starting at index 79 would span 79..82
        let s = "# 用 script 模拟 TTY 交互来添加 DeepSeek 提供商\n# 首先看看有哪些";
        let result = truncate_utf8(s, 80);
        assert!(result.len() <= 80);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn truncate_utf8_empty() {
        assert_eq!(truncate_utf8("", 80), "");
    }

    #[test]
    fn truncate_utf8_zero_limit() {
        assert_eq!(truncate_utf8("hello", 0), "");
    }

    #[test]
    fn truncate_utf8_emoji() {
        // 🦀 is 4 bytes
        let s = "aaa🦀bbb";
        assert_eq!(truncate_utf8(s, 4), "aaa");
        assert_eq!(truncate_utf8(s, 7), "aaa🦀");
    }
}
