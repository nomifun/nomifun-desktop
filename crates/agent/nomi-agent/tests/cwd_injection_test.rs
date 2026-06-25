//! Integration tests verifying that tools use the injected workspace cwd
//! rather than the process working directory.

use std::fs;

use nomi_tools::Tool;
use nomi_tools::bash::BashTool;
use nomi_tools::glob::GlobTool;
use nomi_tools::grep::GrepTool;
use serde_json::json;
use tempfile::tempdir;

// Windows `cd` outputs 8.3 short names (RUNNER~1) that don't match canonicalized paths;
// bash_tool_with_file_operations_uses_correct_cwd covers the same behavior reliably.
#[cfg(not(windows))]
#[tokio::test]
async fn bash_tool_executes_in_injected_cwd_not_process_cwd() {
    let workspace = tempdir().unwrap();
    let tool = BashTool::new(workspace.path().to_path_buf());

    let result = tool.execute(json!({"command": "pwd"})).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let expected = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());
    assert!(
        result.content.contains(expected.to_string_lossy().as_ref()),
        "BashTool should run in injected cwd '{}', got: {}",
        expected.display(),
        result.content
    );
}

#[tokio::test]
async fn glob_tool_finds_files_relative_to_injected_cwd() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("cwd_marker.txt"), "hello").unwrap();

    let tool = GlobTool::new(workspace.path().to_path_buf());
    let result = tool.execute(json!({"pattern": "cwd_marker.txt"})).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(
        result.content.contains("cwd_marker.txt"),
        "GlobTool should find file relative to injected cwd, got: {}",
        result.content
    );
}

#[tokio::test]
async fn grep_tool_searches_relative_to_injected_cwd() {
    let workspace = tempdir().unwrap();
    fs::write(
        workspace.path().join("searchable.txt"),
        "unique_cwd_injection_marker_99",
    )
    .unwrap();

    let tool = GrepTool::new(workspace.path().to_path_buf());
    let result = tool
        .execute(json!({"pattern": "unique_cwd_injection_marker_99", "path": "."}))
        .await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(
        result.content.contains("unique_cwd_injection_marker_99"),
        "GrepTool should search in injected cwd, got: {}",
        result.content
    );
}

#[tokio::test]
async fn bash_tool_with_file_operations_uses_correct_cwd() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("canary.txt"), "found_it").unwrap();

    let tool = BashTool::new(workspace.path().to_path_buf());
    let result = tool.execute(json!({"command": "cat canary.txt"})).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(
        result.content.contains("found_it"),
        "BashTool should be able to read files in injected cwd, got: {}",
        result.content
    );
}
