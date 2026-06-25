use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_types::file_state::FileState;
use nomi_types::tool::{JsonSchema, ToolImage, ToolResult};

use crate::Tool;
use crate::file_cache::{FileStateCache, file_mtime_ms};

/// Stub returned when a file has not changed since the model last read it.
/// Saves tokens by avoiding re-sending identical content.
const FILE_UNCHANGED_STUB: &str = "File unchanged since last read. The content from the earlier Read \
     tool_result in this conversation is still current — refer to that \
     instead of re-reading.";

/// Image read returns the bytes to the LLM as a ToolImage instead of the
/// "(binary file)" stub. Capped so a huge image cannot blow up the request.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// MIME type for image extensions the LLM API accepts as image content blocks
/// (jpeg/png/gif/webp). bmp/tiff keep the binary stub; svg is text and is read
/// as source like any text file.
fn image_media_type(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub struct ReadTool {
    file_cache: Option<Arc<RwLock<FileStateCache>>>,
    /// Session working directory used to resolve relative `file_path` inputs
    /// (matching Grep/Glob/Bash). `None` leaves relative paths resolving
    /// against the process cwd (legacy behavior).
    cwd: Option<PathBuf>,
}

impl ReadTool {
    /// Create a ReadTool with optional file state cache for dedup and an
    /// optional session cwd for resolving relative paths.
    ///
    /// Pass `None` for `file_cache` to disable caching (all reads return full
    /// content). Pass `None` for `cwd` to keep relative paths resolving against
    /// the process working directory.
    pub fn new(file_cache: Option<Arc<RwLock<FileStateCache>>>, cwd: Option<PathBuf>) -> Self {
        Self { file_cache, cwd }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Reads a file from the local filesystem. Returns content with line numbers.\n\n\
         Usage:\n\
         - Prefer an absolute path for file_path; a relative path is resolved against the session working directory.\n\
         - By default, it reads the entire file. Use offset and limit for partial reads on large files.\n\
         - Results are returned with line numbers (1-based) followed by a tab and the line content.\n\
         - Image files (jpg/png/gif/webp) are returned as viewable images. Other binary files return \"(binary file, N bytes)\".\n\
         - This tool can only read files, not directories. To list a directory, use Bash with ls."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to read (absolute preferred; a relative path resolves against the session working directory)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(raw_path) = input["file_path"].as_str() else {
            return ToolResult {
                content: "Missing required parameter: file_path".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };

        // Relative paths resolve against the session working directory when one
        // was injected (matching Grep/Glob/Bash). Everything below — including
        // the cache key — uses the resolved path so dedup stays consistent.
        let resolved: String = match &self.cwd {
            Some(cwd) if !Path::new(raw_path).is_absolute() => cwd.join(raw_path).to_string_lossy().into_owned(),
            _ => raw_path.to_owned(),
        };
        let file_path = resolved.as_str();

        let offset = input["offset"].as_u64().map(|v| v as usize);
        let limit = input["limit"].as_u64().map(|v| v as usize);

        // Get file mtime for dedup and cache.
        let mtime_ms = file_mtime_ms(Path::new(file_path));

        // Dedup check: if cache has the same file with matching offset/limit and mtime,
        // return a short stub instead of full content.
        if let (Some(cache_arc), Some(current_mtime)) = (&self.file_cache, mtime_ms)
            && let Ok(mut cache) = cache_arc.write()
            && let Some(cached) = cache.get(Path::new(file_path))
            && cached.offset == offset
            && cached.limit == limit
            && cached.mtime_ms == current_mtime
        {
            return ToolResult {
                content: FILE_UNCHANGED_STUB.to_string(),
                is_error: false,
                images: Vec::new(),
            };
        }

        // Read file from disk.
        let content = match std::fs::read(file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to read file {}: {}", file_path, e),
                    is_error: true,
                    images: Vec::new(),
                };
            }
        };

        // Image files come back as a multimodal result via ToolResult.images —
        // the same channel screenshots use. Other binaries keep the stub below.
        if let Some(media_type) = image_media_type(file_path) {
            if content.len() > MAX_IMAGE_BYTES {
                return ToolResult {
                    content: format!(
                        "(image file too large to attach: {} bytes, max {} bytes)",
                        content.len(),
                        MAX_IMAGE_BYTES
                    ),
                    is_error: false,
                    images: Vec::new(),
                };
            }
            let data = base64::engine::general_purpose::STANDARD.encode(&content);
            return ToolResult {
                content: format!("(image: {}, {} bytes, {})", file_path, content.len(), media_type),
                is_error: false,
                images: vec![ToolImage {
                    media_type: media_type.to_string(),
                    data,
                }],
            };
        }

        // Check if binary.
        if content.iter().take(8192).any(|&b| b == 0) {
            return ToolResult {
                content: format!("(binary file, {} bytes)", content.len()),
                is_error: false,
                images: Vec::new(),
            };
        }

        let text = String::from_utf8_lossy(&content);
        let lines: Vec<&str> = text.lines().collect();

        let effective_offset = offset.unwrap_or(0);
        let effective_limit = limit.unwrap_or(lines.len());

        let end = (effective_offset + effective_limit).min(lines.len());
        let slice = &lines[effective_offset.min(lines.len())..end];

        let numbered: Vec<String> = slice
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", effective_offset + i + 1, line))
            .collect();

        let result_content = numbered.join("\n");

        // Update cache after successful read.
        if let Some(cache_arc) = &self.file_cache
            && let (Ok(mut cache), Some(mtime)) = (cache_arc.write(), mtime_ms)
        {
            cache.insert(
                file_path.into(),
                FileState {
                    content: result_content.clone(),
                    mtime_ms: mtime,
                    offset,
                    limit,
                },
            );
        }

        ToolResult {
            content: result_content,
            is_error: false,
            images: Vec::new(),
        }
    }

    fn max_result_size(&self) -> usize {
        100_000
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn describe(&self, input: &Value) -> String {
        let path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        format!("Read {}", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::tempdir;

    use nomi_config::file_cache::FileCacheConfig;

    fn make_cache() -> Arc<RwLock<FileStateCache>> {
        let config = FileCacheConfig {
            max_entries: 100,
            max_size_bytes: 25 * 1024 * 1024,
            enabled: true,
        };
        Arc::new(RwLock::new(FileStateCache::new(&config)))
    }

    // -- Basic read tests (no cache) --

    #[tokio::test]
    async fn test_read_file_full() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "line one").unwrap();
        writeln!(file, "line two").unwrap();
        writeln!(file, "line three").unwrap();
        drop(file);

        let tool = ReadTool::new(None, None);
        let input = json!({ "file_path": file_path.to_str().unwrap() });
        let result = tool.execute(input).await;

        assert!(!result.is_error);
        assert!(result.content.contains("1\tline one"));
        assert!(result.content.contains("2\tline two"));
        assert!(result.content.contains("3\tline three"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_and_limit() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("lines.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1..=10 {
            writeln!(file, "line {}", i).unwrap();
        }
        drop(file);

        let tool = ReadTool::new(None, None);
        let input = json!({
            "file_path": file_path.to_str().unwrap(),
            "offset": 2,
            "limit": 3
        });
        let result = tool.execute(input).await;

        assert!(!result.is_error);
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("3\tline 3"));
        assert!(lines[1].contains("4\tline 4"));
        assert!(lines[2].contains("5\tline 5"));
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let tool = ReadTool::new(None, None);
        let input = json!({ "file_path": "/tmp/nonexistent_file_abc123.txt" });
        let result = tool.execute(input).await;

        assert!(result.is_error);
        assert!(result.content.contains("Failed to read file"));
    }

    #[tokio::test]
    async fn test_read_empty_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        std::fs::File::create(&file_path).unwrap();

        let tool = ReadTool::new(None, None);
        let input = json!({ "file_path": file_path.to_str().unwrap() });
        let result = tool.execute(input).await;

        assert!(!result.is_error);
        assert!(result.content.is_empty());
    }

    #[tokio::test]
    async fn test_read_large_file_truncation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("large.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1..=200 {
            writeln!(file, "line number {}", i).unwrap();
        }
        drop(file);

        let tool = ReadTool::new(None, None);
        let input = json!({ "file_path": file_path.to_str().unwrap() });
        let result = tool.execute(input).await;

        assert!(!result.is_error);
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines.len(), 200);
        assert!(lines[0].contains("1\tline number 1"));
        assert!(lines[199].contains("200\tline number 200"));
    }

    // -- Dedup tests (with cache) --

    #[tokio::test]
    async fn dedup_returns_stub_on_unchanged_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("dedup.txt");
        std::fs::write(&file_path, "hello\n").unwrap();

        let cache = make_cache();
        let tool = ReadTool::new(Some(cache), None);

        let input = json!({ "file_path": file_path.to_str().unwrap() });

        // First read: full content.
        let r1 = tool.execute(input.clone()).await;
        assert!(!r1.is_error);
        assert!(r1.content.contains("hello"));

        // Second read: dedup stub.
        let r2 = tool.execute(input).await;
        assert!(!r2.is_error);
        assert_eq!(r2.content, FILE_UNCHANGED_STUB);
    }

    #[tokio::test]
    async fn dedup_returns_new_content_after_modification() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("modified.txt");
        std::fs::write(&file_path, "version1\n").unwrap();

        let cache = make_cache();
        let tool = ReadTool::new(Some(cache), None);

        let input = json!({ "file_path": file_path.to_str().unwrap() });

        let r1 = tool.execute(input.clone()).await;
        assert!(r1.content.contains("version1"));

        // Modify the file — ensure mtime changes.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&file_path, "version2\n").unwrap();

        let r2 = tool.execute(input).await;
        assert!(!r2.is_error);
        assert!(r2.content.contains("version2"));
    }

    #[tokio::test]
    async fn dedup_different_offset_limit_returns_full() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("multi.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1..=20 {
            writeln!(file, "line {}", i).unwrap();
        }
        drop(file);

        let cache = make_cache();
        let tool = ReadTool::new(Some(cache), None);

        let input1 = json!({
            "file_path": file_path.to_str().unwrap(),
            "offset": 0,
            "limit": 10
        });
        let r1 = tool.execute(input1).await;
        assert!(!r1.is_error);
        assert!(r1.content.contains("line 1"));

        // Different range: should return full content, not stub.
        let input2 = json!({
            "file_path": file_path.to_str().unwrap(),
            "offset": 10,
            "limit": 10
        });
        let r2 = tool.execute(input2).await;
        assert!(!r2.is_error);
        assert!(r2.content.contains("line 11"));
        assert!(!r2.content.contains(FILE_UNCHANGED_STUB));
    }

    #[tokio::test]
    async fn no_cache_always_returns_full_content() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("nocache.txt");
        std::fs::write(&file_path, "data\n").unwrap();

        let tool = ReadTool::new(None, None);
        let input = json!({ "file_path": file_path.to_str().unwrap() });

        let r1 = tool.execute(input.clone()).await;
        assert!(r1.content.contains("data"));

        let r2 = tool.execute(input).await;
        assert!(r2.content.contains("data"));
        assert_ne!(r2.content, FILE_UNCHANGED_STUB);
    }

    #[tokio::test]
    async fn nonexistent_file_not_cached() {
        let cache = make_cache();
        let tool = ReadTool::new(Some(cache.clone()), None);

        let input = json!({ "file_path": "/tmp/nonexistent_xyz_789.txt" });
        let r = tool.execute(input).await;
        assert!(r.is_error);

        // Cache should be empty.
        let c = cache.read().unwrap();
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn dedup_empty_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        std::fs::File::create(&file_path).unwrap();

        let cache = make_cache();
        let tool = ReadTool::new(Some(cache), None);

        let input = json!({ "file_path": file_path.to_str().unwrap() });

        let r1 = tool.execute(input.clone()).await;
        assert!(!r1.is_error);

        let r2 = tool.execute(input).await;
        assert!(!r2.is_error);
        assert_eq!(r2.content, FILE_UNCHANGED_STUB);
    }

    // -- Image branch tests --

    /// Minimal valid PNG header bytes (enough to be a "binary" file with NUL bytes).
    const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D];

    #[tokio::test]
    async fn read_png_returns_multimodal_image() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("pic.png");
        std::fs::write(&file_path, PNG_BYTES).unwrap();

        let tool = ReadTool::new(None, None);
        let result = tool.execute(json!({ "file_path": file_path.to_str().unwrap() })).await;

        assert!(!result.is_error);
        assert_eq!(result.images.len(), 1, "png must come back as a ToolImage");
        assert_eq!(result.images[0].media_type, "image/png");
        assert!(!result.images[0].data.is_empty());
        assert!(result.content.contains("image"), "text content should describe the image");
        assert!(!result.content.contains("(binary file"), "image must not fall through to the binary stub");
    }

    #[tokio::test]
    async fn read_oversized_image_returns_hint_without_image() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("big.png");
        // > 5MB of zeroes
        std::fs::write(&file_path, vec![0u8; 5 * 1024 * 1024 + 1]).unwrap();

        let tool = ReadTool::new(None, None);
        let result = tool.execute(json!({ "file_path": file_path.to_str().unwrap() })).await;

        assert!(!result.is_error);
        assert!(result.images.is_empty());
        assert!(result.content.contains("too large"));
    }

    #[tokio::test]
    async fn read_non_image_binary_unchanged() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("blob.bin");
        std::fs::write(&file_path, [0u8, 1, 2, 3]).unwrap();

        let tool = ReadTool::new(None, None);
        let result = tool.execute(json!({ "file_path": file_path.to_str().unwrap() })).await;

        assert!(!result.is_error);
        assert!(result.images.is_empty());
        assert!(result.content.contains("(binary file"));
    }

    // -- Relative-path resolution (session cwd) --

    #[tokio::test]
    async fn relative_path_resolves_against_cwd() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("rel.txt"), "cwd resolved content\n").unwrap();

        let tool = ReadTool::new(None, Some(dir.path().to_path_buf()));
        let result = tool.execute(json!({ "file_path": "rel.txt" })).await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("cwd resolved content"));
    }

    #[tokio::test]
    async fn relative_subdir_path_resolves_against_cwd() {
        // The prompt hands the model paths like "./.nomi/requirement-attachments/…" —
        // a ./-prefixed nested relative path must resolve under the session cwd.
        let dir = tempdir().unwrap();
        let sub = dir.path().join(".nomi").join("requirement-attachments");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("note.txt"), "staged attachment\n").unwrap();

        let tool = ReadTool::new(None, Some(dir.path().to_path_buf()));
        let result = tool
            .execute(json!({ "file_path": "./.nomi/requirement-attachments/note.txt" }))
            .await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("staged attachment"));
    }

    #[tokio::test]
    async fn relative_path_without_cwd_fails_gracefully() {
        // No cwd injected → legacy behavior: the relative path is tried as-is
        // against the process cwd; a missing file is a read failure, not a panic.
        let tool = ReadTool::new(None, None);
        let result = tool
            .execute(json!({ "file_path": "definitely_missing_rel_file_xyz_42.txt" }))
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("Failed to read file"));
    }

    #[tokio::test]
    async fn relative_path_dedup_uses_resolved_cache_key() {
        // The cache key must be the RESOLVED path so dedup behaves identically
        // whether the model passes the relative or the absolute form.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("rel.txt"), "hello\n").unwrap();

        let cache = make_cache();
        let tool = ReadTool::new(Some(cache), Some(dir.path().to_path_buf()));

        let r1 = tool.execute(json!({ "file_path": "rel.txt" })).await;
        assert!(r1.content.contains("hello"));

        // Same file via its absolute path → same cache entry → dedup stub.
        let abs = dir.path().join("rel.txt");
        let r2 = tool.execute(json!({ "file_path": abs.to_str().unwrap() })).await;
        assert!(!r2.is_error);
        assert_eq!(r2.content, FILE_UNCHANGED_STUB);
    }
}
