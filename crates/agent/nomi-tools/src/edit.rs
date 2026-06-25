use std::path::Path;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::Tool;
use crate::file_cache::{FileStateCache, file_mtime_ms, update_cache_after_write};

/// A single find/replace operation within a file.
pub(crate) struct EditOp {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

/// Apply a sequence of edits to `content` in order, returning the new content
/// and the total number of replacements. All-or-nothing: if any hunk fails to
/// match (or is ambiguous without `replace_all`), returns `Err` and the caller
/// MUST NOT write — so a multi-edit never leaves a file partially modified.
/// Later hunks see the text produced by earlier ones (sequential semantics).
/// Single-edit error messages stay unprefixed for backward compatibility.
pub(crate) fn apply_edits(content: &str, ops: &[EditOp]) -> Result<(String, usize), String> {
    let multi = ops.len() > 1;
    let mut current = content.to_string();
    let mut total = 0usize;
    for (i, op) in ops.iter().enumerate() {
        let label = if multi { format!("edit #{}: ", i + 1) } else { String::new() };
        let count = current.matches(&op.old_string).count();
        if count == 0 {
            return Err(format!("{label}old_string not found in file"));
        }
        if count > 1 && !op.replace_all {
            return Err(format!(
                "{label}Multiple matches found ({count}). Use replace_all or provide more context."
            ));
        }
        current = if op.replace_all {
            current.replace(&op.old_string, &op.new_string)
        } else {
            current.replacen(&op.old_string, &op.new_string, 1)
        };
        total += if op.replace_all { count } else { 1 };
    }
    Ok((current, total))
}

pub struct EditTool {
    file_cache: Option<Arc<RwLock<FileStateCache>>>,
    /// Optional containment root; when set, edits outside it are rejected.
    write_root: Option<std::path::PathBuf>,
}

impl EditTool {
    /// Create an EditTool with optional file state cache.
    ///
    /// When cache is `Some`, the tool enforces:
    /// - "Must Read first" guard (file must be in cache before editing)
    /// - Staleness detection (disk mtime must match cached mtime)
    /// - Post-write cache update (mtime + content refreshed after edit)
    ///
    /// Pass `None` to disable all cache-related guards (legacy behavior).
    pub fn new(file_cache: Option<Arc<RwLock<FileStateCache>>>) -> Self {
        Self {
            file_cache,
            write_root: None,
        }
    }

    /// Restrict edits to within `root` (design §3.6 write-root containment).
    pub fn with_write_root(mut self, root: Option<std::path::PathBuf>) -> Self {
        self.write_root = root;
        self
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Performs exact string replacements in files.\n\n\
         Usage:\n\
         - You must use the Read tool first before editing a file.\n\
         - For a single change, pass old_string + new_string.\n\
         - To change several places in ONE file in a single call, pass an `edits` \
         array of {old_string, new_string, replace_all?} objects — they are applied \
         in order, atomically (all or nothing): if any hunk fails to match, the file \
         is left untouched. Prefer this over many separate Edit calls when refactoring.\n\
         - Each old_string must be unique in the file (at the point it is applied). \
         If multiple matches exist, the edit fails — add surrounding context or set \
         replace_all to change every occurrence.\n\
         - Prefer Edit over Write for modifying existing files — Edit only sends the diff.\n\
         - When matching text from Read output, preserve the exact indentation (tabs/spaces)."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to replace (single-edit mode)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text (single-edit mode)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)"
                },
                "edits": {
                    "type": "array",
                    "description": "Multi-edit mode: a list of edits applied in order to the same file, atomically (all-or-nothing). Use instead of old_string/new_string for multiple changes in one call.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string", "description": "The text to replace" },
                            "new_string": { "type": "string", "description": "The replacement text" },
                            "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(file_path) = input["file_path"].as_str() else {
            return ToolResult {
                content: "Missing required parameter: file_path".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };

        // Write-root containment (opt-in): reject edits outside the configured root.
        if let Some(msg) = crate::path_guard::ensure_within_root(file_path, self.write_root.as_deref()) {
            return ToolResult {
                content: msg,
                is_error: true,
                images: Vec::new(),
            };
        }

        // Accept either a multi-edit `edits` array (applied atomically in one
        // write) or the legacy single old_string/new_string triple.
        let ops: Vec<EditOp> = if let Some(arr) = input["edits"].as_array() {
            if arr.is_empty() {
                return ToolResult {
                    content: "edits array must not be empty".to_string(),
                    is_error: true,
                    images: Vec::new(),
                };
            }
            let mut ops = Vec::with_capacity(arr.len());
            for (i, e) in arr.iter().enumerate() {
                let (Some(o), Some(n)) = (e["old_string"].as_str(), e["new_string"].as_str()) else {
                    return ToolResult {
                        content: format!("edit #{}: missing old_string or new_string", i + 1),
                        is_error: true,
                        images: Vec::new(),
                    };
                };
                ops.push(EditOp {
                    old_string: o.to_string(),
                    new_string: n.to_string(),
                    replace_all: e["replace_all"].as_bool().unwrap_or(false),
                });
            }
            ops
        } else {
            let Some(old_string) = input["old_string"].as_str() else {
                return ToolResult {
                    content: "Missing required parameter: old_string".to_string(),
                    is_error: true,
                    images: Vec::new(),
                };
            };
            let Some(new_string) = input["new_string"].as_str() else {
                return ToolResult {
                    content: "Missing required parameter: new_string".to_string(),
                    is_error: true,
                    images: Vec::new(),
                };
            };
            vec![EditOp {
                old_string: old_string.to_string(),
                new_string: new_string.to_string(),
                replace_all: input["replace_all"].as_bool().unwrap_or(false),
            }]
        };

        let path = Path::new(file_path);

        // Cache guard: "must Read first" + staleness detection.
        if let Some(cache_arc) = &self.file_cache
            && let Ok(mut cache) = cache_arc.write()
        {
            let cached = cache.get(path);
            if cached.is_none() {
                return ToolResult {
                    content: format!(
                        "You must Read {} before editing. Use the Read tool first \
                         so the file content is loaded into context.",
                        file_path
                    ),
                    is_error: true,
                    images: Vec::new(),
                };
            }
            // Staleness check: compare cached mtime with current disk mtime.
            let cached_mtime = cached.map(|s| s.mtime_ms);
            let disk_mtime = file_mtime_ms(path);
            if let (Some(cached_mt), Some(disk_mt)) = (cached_mtime, disk_mtime)
                && cached_mt != disk_mt
            {
                return ToolResult {
                    content: format!(
                        "File {} has been modified externally since last read. \
                         Read the file again to see the current content before editing.",
                        file_path
                    ),
                    is_error: true,
                    images: Vec::new(),
                };
            }
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to read file {}: {}", file_path, e),
                    is_error: true,
                    images: Vec::new(),
                };
            }
        };

        let (new_content, total) = match apply_edits(&content, &ops) {
            Ok(r) => r,
            Err(msg) => {
                return ToolResult {
                    content: msg,
                    is_error: true,
                    images: Vec::new(),
                };
            }
        };

        if let Err(e) = crate::atomic_write(file_path, &new_content) {
            return ToolResult {
                content: format!("Failed to write file: {}", e),
                is_error: true,
                images: Vec::new(),
            };
        }

        // Post-write cache update: refresh mtime and content.
        if let Some(cache_arc) = &self.file_cache {
            update_cache_after_write(cache_arc, path, &new_content);
        }

        ToolResult {
            content: if ops.len() > 1 {
                format!(
                    "Edited {}: {} replacement(s) across {} edits",
                    file_path,
                    total,
                    ops.len()
                )
            } else {
                format!("Edited {}: replaced {} occurrence(s)", file_path, total)
            },
            is_error: false,
            images: Vec::new(),
        }
    }

    fn max_result_size(&self) -> usize {
        10_000
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn describe(&self, input: &Value) -> String {
        let path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        format!("Edit {}", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::file_cache::update_cache_after_write;
    use nomi_config::file_cache::FileCacheConfig;

    fn make_cache() -> Arc<RwLock<FileStateCache>> {
        let config = FileCacheConfig {
            max_entries: 100,
            max_size_bytes: 25 * 1024 * 1024,
            enabled: true,
        };
        Arc::new(RwLock::new(FileStateCache::new(&config)))
    }

    /// Simulate a Read by inserting a cache entry for the given file path.
    fn simulate_read(cache: &Arc<RwLock<FileStateCache>>, path: &Path) {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        update_cache_after_write(cache, path, &content);
    }

    #[tokio::test]
    async fn multi_edit_applies_all_hunks_in_one_call() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("m.txt");
        std::fs::write(&file_path, "alpha beta gamma").unwrap();
        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "edits": [
                { "old_string": "alpha", "new_string": "A" },
                { "old_string": "gamma", "new_string": "G" }
            ]
        });
        let result = tool.execute(input).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "A beta G");
    }

    #[tokio::test]
    async fn multi_edit_failing_hunk_leaves_file_untouched() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("m.txt");
        std::fs::write(&file_path, "alpha beta").unwrap();
        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "edits": [
                { "old_string": "alpha", "new_string": "A" },
                { "old_string": "NOPE", "new_string": "x" }
            ]
        });
        let result = tool.execute(input).await;
        assert!(result.is_error, "a failing hunk must fail the whole edit");
        // Atomic: the first (matching) hunk must NOT have been written.
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "alpha beta");
    }

    // -- Legacy tests (no cache) --

    #[test]
    fn apply_edits_applies_multiple_hunks_in_order() {
        use super::{EditOp, apply_edits};
        let ops = vec![
            EditOp { old_string: "foo".into(), new_string: "bar".into(), replace_all: false },
            EditOp { old_string: "bar".into(), new_string: "baz".into(), replace_all: false },
        ];
        // Sequential: edit 1 foo->bar => "bar X"; edit 2 sees "bar" and -> baz => "baz X".
        let (out, n) = apply_edits("foo X", &ops).unwrap();
        assert_eq!(out, "baz X");
        assert_eq!(n, 2);
    }

    #[test]
    fn apply_edits_aborts_on_missing_hunk_identifying_which() {
        use super::{EditOp, apply_edits};
        let ops = vec![
            EditOp { old_string: "foo".into(), new_string: "bar".into(), replace_all: false },
            EditOp { old_string: "NOPE".into(), new_string: "x".into(), replace_all: false },
        ];
        let err = apply_edits("foo", &ops).unwrap_err();
        assert!(err.contains("not found"));
        assert!(err.contains("edit #2"), "must identify the failing hunk: {err}");
    }

    #[test]
    fn apply_edits_replace_all_counts_all_occurrences() {
        use super::{EditOp, apply_edits};
        let ops = vec![EditOp { old_string: "a".into(), new_string: "b".into(), replace_all: true }];
        let (out, n) = apply_edits("a a a", &ops).unwrap();
        assert_eq!(out, "b b b");
        assert_eq!(n, 3);
    }

    #[test]
    fn apply_edits_single_hunk_messages_unprefixed() {
        use super::{EditOp, apply_edits};
        // A single edit keeps the legacy unprefixed error message (back-compat).
        let ops = vec![EditOp { old_string: "x".into(), new_string: "y".into(), replace_all: false }];
        let err = apply_edits("no match here", &ops).unwrap_err();
        assert_eq!(err, "old_string not found in file");
    }

    #[test]
    fn atomic_write_creates_and_replaces_without_leftover_temp() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("f.txt");
        let ps = p.to_str().unwrap();

        crate::atomic_write(ps, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");

        crate::atomic_write(ps, "world").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "world");

        // The temp file must be renamed onto the target, never left behind.
        let leftover = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover, "atomic_write must rename the temp file away");
    }

    #[tokio::test]
    async fn test_edit_replace_block() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        });

        let result = tool.execute(input).await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_old_string_not_found() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "nonexistent",
            "new_string": "replacement"
        });

        let result = tool.execute(input).await;

        assert!(result.is_error);
        assert!(
            result.content.contains("not found"),
            "expected 'not found' in error message, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_edit_preserves_surrounding() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "aaa\nbbb\nccc\n").unwrap();

        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "bbb",
            "new_string": "XXX"
        });

        let result = tool.execute(input).await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "aaa\nXXX\nccc\n");
    }

    #[tokio::test]
    async fn test_edit_nonexistent_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("does_not_exist.txt");

        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "anything",
            "new_string": "replacement"
        });

        let result = tool.execute(input).await;

        assert!(result.is_error);
        assert!(
            result.content.contains("Failed to read file"),
            "expected read failure message, got: {}",
            result.content
        );
    }

    // -- Cache guard tests --

    #[tokio::test]
    async fn edit_without_read_returns_error() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("unread.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let cache = make_cache();
        let tool = EditTool::new(Some(cache));

        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "bye"
        });

        let result = tool.execute(input).await;

        assert!(result.is_error);
        assert!(
            result.content.contains("must Read"),
            "expected 'must Read' in error: {}",
            result.content
        );
        // File must be unchanged.
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello");
    }

    #[tokio::test]
    async fn edit_after_read_succeeds() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("read_then_edit.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let cache = make_cache();
        simulate_read(&cache, &file_path);

        let tool = EditTool::new(Some(cache));
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        });

        let result = tool.execute(input).await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "goodbye world"
        );
    }

    #[tokio::test]
    async fn edit_detects_external_modification() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("stale.txt");
        std::fs::write(&file_path, "original").unwrap();

        let cache = make_cache();
        simulate_read(&cache, &file_path);

        // External modification: change file after caching.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&file_path, "externally changed").unwrap();

        let tool = EditTool::new(Some(cache));
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "original",
            "new_string": "new"
        });

        let result = tool.execute(input).await;

        assert!(result.is_error);
        assert!(
            result.content.contains("modified externally"),
            "expected staleness error: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn edit_then_edit_succeeds_via_cache_update() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("double_edit.txt");
        std::fs::write(&file_path, "aaa bbb ccc").unwrap();

        let cache = make_cache();
        simulate_read(&cache, &file_path);

        let tool = EditTool::new(Some(cache));

        // First edit.
        let input1 = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "aaa",
            "new_string": "AAA"
        });
        let r1 = tool.execute(input1).await;
        assert!(!r1.is_error, "first edit failed: {}", r1.content);

        // Second edit should succeed because first edit updated the cache.
        let input2 = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "bbb",
            "new_string": "BBB"
        });
        let r2 = tool.execute(input2).await;
        assert!(!r2.is_error, "second edit failed: {}", r2.content);
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "AAA BBB ccc");
    }

    #[tokio::test]
    async fn no_cache_edit_bypasses_guard() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("nocache.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let tool = EditTool::new(None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "bye"
        });

        let result = tool.execute(input).await;
        assert!(
            !result.is_error,
            "expected success without cache: {}",
            result.content
        );
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "bye");
    }

    #[tokio::test]
    async fn replace_all_updates_cache() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("replaceall.txt");
        std::fs::write(&file_path, "a-a-a").unwrap();

        let cache = make_cache();
        simulate_read(&cache, &file_path);

        let tool = EditTool::new(Some(cache.clone()));
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "a",
            "new_string": "b",
            "replace_all": true
        });

        let result = tool.execute(input).await;
        assert!(!result.is_error, "replace_all failed: {}", result.content);

        // Verify cache was updated: mtime should match current disk mtime.
        let disk_mtime = file_mtime_ms(&file_path).unwrap();
        let mut c = cache.write().unwrap();
        let cached = c.get(&file_path).expect("file should be in cache");
        assert_eq!(cached.mtime_ms, disk_mtime);
    }
}
