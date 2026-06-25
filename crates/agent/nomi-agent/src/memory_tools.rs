//! Native `remember` tool: lets the in-process agent persist a durable memory
//! (project fact / user preference / lesson) to the file-based long-term memory
//! mid-session, so it is injected into future sessions' system prompt. Fully
//! self-contained in the agent layer (writes via `nomi_memory`); the host wires
//! only the target directory at bootstrap. Closes the audit's "in-session
//! incremental memory persistence" gap (Claude Code parity).

use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_memory::index::append_index_entry;
use nomi_memory::paths::ENTRYPOINT_NAME;
use nomi_memory::store::write_memory;
use nomi_memory::types::{MemoryEntry, MemoryFrontmatter, MemoryType};
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

/// Derive a filesystem-safe, kebab-case memory name from a title.
fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    let s: String = s.chars().take(60).collect();
    if s.is_empty() { "memory".to_string() } else { s }
}

/// `remember` — persist a long-term memory to the project's memory directory.
pub struct RememberTool {
    memory_dir: PathBuf,
}

impl RememberTool {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }
}

#[async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        "Persist a durable memory to long-term memory so it is available in FUTURE sessions. \
         Use when you learn something worth keeping about this project or the user — e.g. \
         'this project uses pnpm, not npm', 'always run cargo test before committing', a \
         coding convention, or a correction the user made. Do NOT store secrets, credentials, \
         or transient details. Keep each memory to one focused fact."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Short title (becomes the memory's name)" },
                "content": { "type": "string", "description": "The fact / lesson to remember (markdown)" },
                "type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Memory type (default: project)"
                }
            },
            "required": ["title", "content"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let (Some(title), Some(content)) = (input["title"].as_str(), input["content"].as_str())
        else {
            return ToolResult {
                content: "remember requires: title, content".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };
        if content.trim().is_empty() {
            return ToolResult {
                content: "remember: content must not be empty".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        }
        let memory_type = input["type"]
            .as_str()
            .and_then(|s| MemoryType::from_str(s).ok())
            .unwrap_or(MemoryType::Project);

        let frontmatter = MemoryFrontmatter {
            name: Some(slug(title)),
            description: Some(title.to_string()),
            memory_type: Some(memory_type),
            usage_count: None,
            last_used: None,
        };
        let entry = MemoryEntry::new(frontmatter, content.to_string());

        let path = match write_memory(&self.memory_dir, &entry) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to save memory: {e}"),
                    is_error: true,
                    images: Vec::new(),
                };
            }
        };
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("memory.md")
            .to_string();

        // Best-effort index update (a missing/locked index must not fail the save).
        let index_path = self.memory_dir.join(ENTRYPOINT_NAME);
        let _ = append_index_entry(&index_path, title, &filename, title);

        ToolResult {
            content: format!("Remembered '{title}' ({filename})"),
            is_error: false,
            images: Vec::new(),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn describe(&self, input: &Value) -> String {
        let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("");
        format!("Remember: {}", nomi_tools::truncate_utf8(title, 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn slug_is_kebab_and_bounded() {
        assert_eq!(slug("Use pnpm, not npm!"), "use-pnpm-not-npm");
        assert_eq!(slug("   "), "memory");
        assert!(slug(&"x".repeat(200)).len() <= 60);
    }

    #[tokio::test]
    async fn remember_writes_a_memory_file_and_indexes_it() {
        let dir = tempdir().unwrap();
        let tool = RememberTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({
                "title": "Use pnpm",
                "content": "This project uses pnpm, not npm.",
                "type": "project"
            }))
            .await;
        assert!(!r.is_error, "{}", r.content);

        // A memory file was written and the MEMORY.md index references it.
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(files.iter().any(|f| f.ends_with(".md") && f != ENTRYPOINT_NAME), "memory file written: {files:?}");
        let index = std::fs::read_to_string(dir.path().join(ENTRYPOINT_NAME)).unwrap_or_default();
        assert!(index.contains("Use pnpm"), "index should reference the memory: {index}");
    }

    #[tokio::test]
    async fn remember_rejects_missing_or_empty() {
        let dir = tempdir().unwrap();
        let tool = RememberTool::new(dir.path().to_path_buf());
        assert!(tool.execute(json!({ "title": "x" })).await.is_error);
        assert!(
            tool.execute(json!({ "title": "x", "content": "   " }))
                .await
                .is_error
        );
    }
}
