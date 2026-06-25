// Acceptance tests for file cache dedup and cross-tool integration.
//
// These are LOCAL tests — no LLM call required.

use std::sync::{Arc, RwLock};

use serde_json::json;

use nomi_config::file_cache::FileCacheConfig;
use nomi_tools::Tool;
use nomi_tools::edit::EditTool;
use nomi_tools::file_cache::FileStateCache;
use nomi_tools::read::ReadTool;
use nomi_tools::write::WriteTool;

fn make_cache() -> Arc<RwLock<FileStateCache>> {
    let config = FileCacheConfig::default();
    Arc::new(RwLock::new(FileStateCache::new(&config)))
}

/// TC-A5-01: Read dedup (LOCAL, no LLM).
///
/// Verifies that a second read of an unchanged file returns a short dedup stub
/// instead of re-sending the full content.
#[tokio::test]
async fn read_dedup_returns_stub_on_second_read() {
    let cache = make_cache();
    let read_tool = ReadTool::new(Some(cache.clone()), None);

    // Create a temporary file with known content.
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("dedup_test.txt");
    std::fs::write(&file_path, "line one\nline two\nline three\n").unwrap();
    let path_str = file_path.to_str().unwrap();

    let input = json!({ "file_path": path_str });

    // First read: should return full line-numbered content.
    let r1 = read_tool.execute(input.clone()).await;
    assert!(!r1.is_error, "first read should succeed: {}", r1.content);
    assert!(
        r1.content.contains("1\tline one"),
        "first read should contain line-numbered content, got: {}",
        r1.content
    );
    assert!(
        r1.content.contains("2\tline two"),
        "first read should contain line 2"
    );
    assert!(
        r1.content.contains("3\tline three"),
        "first read should contain line 3"
    );

    // Second read WITHOUT modifying the file: should return dedup stub.
    let r2 = read_tool.execute(input).await;
    assert!(!r2.is_error, "second read should succeed: {}", r2.content);
    assert!(
        r2.content.contains("unchanged since last read"),
        "second read should return dedup stub, got: {}",
        r2.content
    );
}

/// TC-A5-02: Write -> Edit chain (LOCAL, no LLM).
///
/// Verifies that WriteTool populates the cache so EditTool can immediately
/// edit the file without a separate Read call (no "must Read first" error).
#[tokio::test]
async fn write_then_edit_chain_succeeds() {
    let cache = make_cache();
    let write_tool = WriteTool::new(Some(cache.clone()));
    let edit_tool = EditTool::new(Some(cache.clone()));

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("write_edit_chain.txt");
    let path_str = file_path.to_str().unwrap();

    // Write a file via WriteTool.
    let write_result = write_tool
        .execute(json!({
            "file_path": path_str,
            "content": "hello world\n"
        }))
        .await;
    assert!(
        !write_result.is_error,
        "write should succeed: {}",
        write_result.content
    );

    // Immediately edit via EditTool — should NOT get "must Read first" error.
    let edit_result = edit_tool
        .execute(json!({
            "file_path": path_str,
            "old_string": "hello",
            "new_string": "goodbye"
        }))
        .await;
    assert!(
        !edit_result.is_error,
        "edit after write should succeed without 'must Read first' error: {}",
        edit_result.content
    );

    // Verify file content on disk.
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        content, "goodbye world\n",
        "file content should reflect the edit"
    );
}
