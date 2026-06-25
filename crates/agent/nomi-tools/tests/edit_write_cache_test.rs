//! Integration tests for EditTool / WriteTool file-state cache integration
//! (TC-5.4 and TC-5.4-W series).
//!
//! Black-box tests: exercise Edit/Write tools through their public API with
//! a real filesystem and shared FileStateCache, validating "must Read first"
//! guard, staleness detection, and post-write cache updates.

use std::path::Path;
use std::sync::{Arc, RwLock};

use serde_json::json;

use nomi_config::file_cache::FileCacheConfig;
use nomi_tools::Tool;
use nomi_tools::edit::EditTool;
use nomi_tools::file_cache::{FileStateCache, file_mtime_ms};
use nomi_tools::read::ReadTool;
use nomi_tools::write::WriteTool;

fn make_cache() -> Arc<RwLock<FileStateCache>> {
    let config = FileCacheConfig {
        max_entries: 100,
        max_size_bytes: 25 * 1024 * 1024,
        enabled: true,
    };
    Arc::new(RwLock::new(FileStateCache::new(&config)))
}

/// Populate cache by actually reading the file through ReadTool.
async fn read_file(tool: &ReadTool, path: &Path) {
    let input = json!({ "file_path": path.to_str().unwrap() });
    let r = tool.execute(input).await;
    assert!(!r.is_error, "read failed: {}", r.content);
}

const UNCHANGED_MARKER: &str = "File unchanged since last read";

// ==========================================================================
// TC-5.4: EditTool guard and staleness detection
// ==========================================================================

/// TC-5.4-01: Normal Read → Edit succeeds.
#[tokio::test]
async fn tc_5_4_01_read_then_edit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("normal.txt");
    std::fs::write(&file, "hello world").unwrap();

    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);
    let edit_tool = EditTool::new(Some(cache));

    read_file(&read_tool, &file).await;

    let input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "hello",
        "new_string": "goodbye"
    });
    let result = edit_tool.execute(input).await;

    assert!(
        !result.is_error,
        "Edit after Read should succeed: {}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye world");
}

/// TC-5.4-02: Edit without prior Read returns "must Read first" error.
#[tokio::test]
async fn tc_5_4_02_edit_without_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("no_read.txt");
    std::fs::write(&file, "content").unwrap();

    let cache = make_cache();
    let edit_tool = EditTool::new(Some(cache));

    let input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "content",
        "new_string": "new"
    });
    let result = edit_tool.execute(input).await;

    assert!(result.is_error, "Edit without Read should fail");
    assert!(
        result.content.contains("must Read"),
        "Error should mention 'must Read': {}",
        result.content
    );
    // File must be unchanged.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "content");
}

/// TC-5.4-03: External modification after Read triggers staleness error.
#[tokio::test]
async fn tc_5_4_03_external_modification_detected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("stale.txt");
    std::fs::write(&file, "original content").unwrap();

    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);
    let edit_tool = EditTool::new(Some(cache));

    read_file(&read_tool, &file).await;

    // External modification.
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(&file, "externally changed").unwrap();

    let input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "original content",
        "new_string": "new"
    });
    let result = edit_tool.execute(input).await;

    assert!(
        result.is_error,
        "Edit of externally modified file should fail"
    );
    assert!(
        result.content.contains("modified externally"),
        "Error should mention external modification: {}",
        result.content
    );
}

/// TC-5.4-04: Edit → Edit succeeds because first Edit updates the cache.
#[tokio::test]
async fn tc_5_4_04_edit_then_edit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("double.txt");
    std::fs::write(&file, "aaa bbb ccc").unwrap();

    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);
    let edit_tool = EditTool::new(Some(cache));

    read_file(&read_tool, &file).await;

    // First edit.
    let input1 = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "aaa",
        "new_string": "AAA"
    });
    let r1 = edit_tool.execute(input1).await;
    assert!(!r1.is_error, "First edit failed: {}", r1.content);

    // Second edit — should work because first edit updated cache mtime.
    let input2 = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "bbb",
        "new_string": "BBB"
    });
    let r2 = edit_tool.execute(input2).await;
    assert!(!r2.is_error, "Second edit failed: {}", r2.content);

    assert_eq!(std::fs::read_to_string(&file).unwrap(), "AAA BBB ccc");
}

/// TC-5.4-05: With cache disabled (None), Edit works without prior Read.
#[tokio::test]
async fn tc_5_4_05_no_cache_edit_bypasses_guard() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nocache.txt");
    std::fs::write(&file, "hello").unwrap();

    let edit_tool = EditTool::new(None);

    let input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "hello",
        "new_string": "bye"
    });
    let result = edit_tool.execute(input).await;

    assert!(
        !result.is_error,
        "Edit without cache should succeed: {}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "bye");
}

/// TC-5.4-06: replace_all updates cache mtime correctly.
#[tokio::test]
async fn tc_5_4_06_replace_all_updates_cache() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("replaceall.txt");
    std::fs::write(&file, "x-x-x-x").unwrap();

    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);
    let edit_tool = EditTool::new(Some(cache.clone()));

    read_file(&read_tool, &file).await;

    let input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "x",
        "new_string": "y",
        "replace_all": true
    });
    let result = edit_tool.execute(input).await;
    assert!(!result.is_error, "replace_all failed: {}", result.content);

    assert_eq!(std::fs::read_to_string(&file).unwrap(), "y-y-y-y");

    // Verify cache mtime matches disk.
    let disk_mtime = file_mtime_ms(&file).unwrap();
    let mut c = cache.write().unwrap();
    let cached = c.get(&file).expect("file should be in cache");
    assert_eq!(cached.mtime_ms, disk_mtime);
}

// ==========================================================================
// TC-5.4-W: WriteTool cache update
// ==========================================================================

/// TC-5.4-W01: Write then Read returns "unchanged" (Write populates cache).
#[tokio::test]
async fn tc_5_4_w01_write_then_read_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("write_read.txt");

    let cache = make_cache();
    let write_tool = WriteTool::new(Some(cache.clone()));
    let read_tool = ReadTool::new(Some(cache), None);

    // Write creates file and populates cache.
    let write_input = json!({
        "file_path": file.to_str().unwrap(),
        "content": "written content"
    });
    let wr = write_tool.execute(write_input).await;
    assert!(!wr.is_error, "write failed: {}", wr.content);

    // Read immediately after: should return "unchanged" because Write
    // already cached the content with the correct mtime.
    let read_input = json!({ "file_path": file.to_str().unwrap() });
    let rr = read_tool.execute(read_input).await;
    assert!(!rr.is_error);
    assert!(
        rr.content.contains(UNCHANGED_MARKER),
        "Read after Write should return unchanged stub, got: {}",
        rr.content
    );
}

/// TC-5.4-W02: Write then Edit succeeds (Write populates cache for Edit guard).
#[tokio::test]
async fn tc_5_4_w02_write_then_edit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("write_edit.txt");

    let cache = make_cache();
    let write_tool = WriteTool::new(Some(cache.clone()));
    let edit_tool = EditTool::new(Some(cache));

    let write_input = json!({
        "file_path": file.to_str().unwrap(),
        "content": "hello world"
    });
    let wr = write_tool.execute(write_input).await;
    assert!(!wr.is_error, "write failed: {}", wr.content);

    let edit_input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "hello",
        "new_string": "goodbye"
    });
    let er = edit_tool.execute(edit_input).await;
    assert!(
        !er.is_error,
        "Edit after Write should succeed: {}",
        er.content
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye world");
}

/// TC-5.4-W03: Write → Write → Read returns fresh content (mtime updated).
#[tokio::test]
async fn tc_5_4_w03_write_overwrite_then_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("overwrite.txt");

    let cache = make_cache();
    let write_tool = WriteTool::new(Some(cache.clone()));
    let read_tool = ReadTool::new(Some(cache), None);

    // First write.
    let w1 = json!({
        "file_path": file.to_str().unwrap(),
        "content": "version 1"
    });
    write_tool.execute(w1).await;

    // Brief delay to change mtime.
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Second write (overwrite).
    let w2 = json!({
        "file_path": file.to_str().unwrap(),
        "content": "version 2"
    });
    write_tool.execute(w2).await;

    // Read: cache was updated by second Write, so should see "unchanged"
    // (cache content matches disk content with matching mtime).
    let read_input = json!({ "file_path": file.to_str().unwrap() });
    let rr = read_tool.execute(read_input).await;
    assert!(!rr.is_error);
    // The cache was updated by the second Write with the new content,
    // so Read should hit the dedup path.
    assert!(
        rr.content.contains(UNCHANGED_MARKER),
        "Read after second Write should dedup, got: {}",
        rr.content
    );

    // Verify disk has version 2.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version 2");
}

// ==========================================================================
// Supplementary: Cross-tool interaction tests
// ==========================================================================

/// Read → Edit → Read should dedup (Edit updated the cache).
#[tokio::test]
async fn read_edit_read_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cross.txt");
    std::fs::write(&file, "alpha beta").unwrap();

    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);
    let edit_tool = EditTool::new(Some(cache));

    // Read.
    read_file(&read_tool, &file).await;

    // Edit.
    let edit_input = json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "alpha",
        "new_string": "ALPHA"
    });
    let er = edit_tool.execute(edit_input).await;
    assert!(!er.is_error);

    // Read again: Edit updated the cache, so Read should see "unchanged".
    let read_input = json!({ "file_path": file.to_str().unwrap() });
    let rr = read_tool.execute(read_input).await;
    assert!(!rr.is_error);
    assert!(
        rr.content.contains(UNCHANGED_MARKER),
        "Read after Edit should dedup, got: {}",
        rr.content
    );
}
