//! Integration tests for ReadTool dedup and cache integration (TC-5.3 series).
//!
//! Black-box tests: exercise ReadTool through its public API with a real
//! filesystem, validating dedup detection and cache update behavior.

use std::sync::{Arc, RwLock};

use serde_json::json;

use nomi_config::file_cache::FileCacheConfig;
use nomi_tools::Tool;
use nomi_tools::file_cache::FileStateCache;
use nomi_tools::read::ReadTool;

fn make_cache() -> Arc<RwLock<FileStateCache>> {
    let config = FileCacheConfig {
        max_entries: 100,
        max_size_bytes: 25 * 1024 * 1024,
        enabled: true,
    };
    Arc::new(RwLock::new(FileStateCache::new(&config)))
}

const UNCHANGED_MARKER: &str = "File unchanged since last read";

/// TC-5.3-01: First read returns full content with line numbers.
#[tokio::test]
async fn tc_5_3_01_first_read_returns_full_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.rs");
    std::fs::write(&file, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let input = json!({ "file_path": file.to_str().unwrap() });
    let result = tool.execute(input).await;

    assert!(!result.is_error);
    assert!(result.content.contains("1\tfn main()"));
    assert!(result.content.contains("2\t    println!"));
    assert!(result.content.contains("3\t}"));
    assert!(
        !result.content.contains(UNCHANGED_MARKER),
        "First read must not return the unchanged stub"
    );
}

/// TC-5.3-02: Second read of the same unchanged file returns the dedup stub.
#[tokio::test]
async fn tc_5_3_02_dedup_on_unchanged_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("stable.txt");
    std::fs::write(&file, "line one\nline two\n").unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let input = json!({ "file_path": file.to_str().unwrap() });

    // First read: full content.
    let r1 = tool.execute(input.clone()).await;
    assert!(!r1.is_error);
    assert!(r1.content.contains("line one"));

    // Second read: unchanged stub.
    let r2 = tool.execute(input).await;
    assert!(!r2.is_error);
    assert!(
        r2.content.contains(UNCHANGED_MARKER),
        "Second read of unchanged file should return the dedup stub"
    );
}

/// TC-5.3-03: After external modification, re-read returns new content.
#[tokio::test]
async fn tc_5_3_03_modified_file_returns_new_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("evolving.txt");
    std::fs::write(&file, "version 1\n").unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let input = json!({ "file_path": file.to_str().unwrap() });

    let r1 = tool.execute(input.clone()).await;
    assert!(r1.content.contains("version 1"));

    // External modification — sleep to ensure mtime changes.
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(&file, "version 2\n").unwrap();

    let r2 = tool.execute(input).await;
    assert!(!r2.is_error);
    assert!(
        r2.content.contains("version 2"),
        "After modification, read should return new content"
    );
    assert!(
        !r2.content.contains(UNCHANGED_MARKER),
        "Modified file must not return unchanged stub"
    );
}

/// TC-5.3-04: Different offset/limit parameters are not deduped.
#[tokio::test]
async fn tc_5_3_04_different_range_no_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multiline.txt");
    let content: String = (1..=30).map(|i| format!("line {}\n", i)).collect();
    std::fs::write(&file, &content).unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let path_str = file.to_str().unwrap();

    // Read lines 0..10.
    let input1 = json!({ "file_path": path_str, "offset": 0, "limit": 10 });
    let r1 = tool.execute(input1).await;
    assert!(!r1.is_error);
    assert!(r1.content.contains("line 1"));

    // Read lines 10..20 — different range, should return full content.
    let input2 = json!({ "file_path": path_str, "offset": 10, "limit": 10 });
    let r2 = tool.execute(input2).await;
    assert!(!r2.is_error);
    assert!(
        r2.content.contains("line 11"),
        "Different offset/limit should return full content"
    );
    assert!(
        !r2.content.contains(UNCHANGED_MARKER),
        "Different range must not trigger dedup"
    );
}

/// TC-5.3-05: With cache disabled (None), reads always return full content.
#[tokio::test]
async fn tc_5_3_05_cache_disabled_no_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nocache.txt");
    std::fs::write(&file, "always full\n").unwrap();

    let tool = ReadTool::new(None, None);
    let input = json!({ "file_path": file.to_str().unwrap() });

    let r1 = tool.execute(input.clone()).await;
    assert!(r1.content.contains("always full"));

    let r2 = tool.execute(input).await;
    assert!(
        r2.content.contains("always full"),
        "Without cache, second read should still return full content"
    );
    assert!(!r2.content.contains(UNCHANGED_MARKER));
}

/// TC-5.3-06: Reading a non-existent file returns an error and does not cache.
#[tokio::test]
async fn tc_5_3_06_nonexistent_file_error_no_cache() {
    let cache = make_cache();
    let tool = ReadTool::new(Some(cache.clone()), None);

    let input = json!({ "file_path": "/tmp/does_not_exist_tc_5_3_06.txt" });
    let result = tool.execute(input).await;

    assert!(result.is_error);
    assert!(result.content.contains("Failed to read file"));

    // Cache should remain empty.
    let c = cache.read().unwrap();
    assert!(c.is_empty(), "Failed reads must not populate the cache");
}

/// TC-5.3-07: Empty file can be deduped on second read.
#[tokio::test]
async fn tc_5_3_07_empty_file_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    std::fs::File::create(&file).unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let input = json!({ "file_path": file.to_str().unwrap() });

    let r1 = tool.execute(input.clone()).await;
    assert!(!r1.is_error);

    let r2 = tool.execute(input).await;
    assert!(!r2.is_error);
    assert!(
        r2.content.contains(UNCHANGED_MARKER),
        "Empty file should be deduped on second read"
    );
}

/// Supplementary: Same range read twice returns dedup stub.
#[tokio::test]
async fn same_range_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("range.txt");
    let content: String = (1..=20).map(|i| format!("line {}\n", i)).collect();
    std::fs::write(&file, &content).unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache), None);

    let input = json!({ "file_path": file.to_str().unwrap(), "offset": 5, "limit": 5 });

    let r1 = tool.execute(input.clone()).await;
    assert!(!r1.is_error);
    assert!(r1.content.contains("line 6"));

    let r2 = tool.execute(input).await;
    assert!(!r2.is_error);
    assert!(
        r2.content.contains(UNCHANGED_MARKER),
        "Same range on unchanged file should trigger dedup"
    );
}

/// Supplementary: Cache entry is updated after modification + re-read.
#[tokio::test]
async fn cache_updated_after_modification() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("update.txt");
    std::fs::write(&file, "v1\n").unwrap();

    let cache = make_cache();
    let tool = ReadTool::new(Some(cache.clone()), None);

    let input = json!({ "file_path": file.to_str().unwrap() });

    // First read: caches v1.
    tool.execute(input.clone()).await;

    // Modify.
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(&file, "v2\n").unwrap();

    // Second read: returns v2 and updates cache.
    let r2 = tool.execute(input.clone()).await;
    assert!(r2.content.contains("v2"));

    // Third read: should dedup on v2.
    let r3 = tool.execute(input).await;
    assert!(
        r3.content.contains(UNCHANGED_MARKER),
        "After re-read of modified file, cache should be updated and third read deduped"
    );
}
