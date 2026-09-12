//! Integration tests for workspace snapshot (tasks 7.9 + 7.10).
//!
//! Tests exercise `ISnapshotService` through `SnapshotService`, verifying
//! both git-repo and snapshot modes for all snapshot operations.

use std::path::Path;

use git2::{Repository, Signature};
use nomifun_common::FileChangeOperation;
use nomifun_file::{ISnapshotService, SnapshotMode, SnapshotService};

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Create a git repo at `path` with an initial commit containing no files.
fn init_empty_repo(path: &Path) {
    let repo = Repository::init(path).expect("init repo");
    let mut index = repo.index().expect("index");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = Signature::now("test", "test@test.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .expect("commit");
}

/// Create a git repo at `path` with an initial commit that tracks a file.
/// Parent directories for nested filenames are created automatically.
fn init_repo_with_file(path: &Path, filename: &str, content: &str) {
    init_repo_with_files(path, &[(filename, content)]);
}

fn init_repo_with_files(path: &Path, files: &[(&str, &str)]) {
    let repo = Repository::init(path).expect("init repo");
    let mut index = repo.index().expect("index");
    for (filename, content) in files {
        let file_path = path.join(filename);
        std::fs::create_dir_all(file_path.parent().unwrap()).expect("create parent dirs");
        std::fs::write(&file_path, content).expect("write file");
        index.add_path(Path::new(filename)).expect("add path");
    }
    index.write().expect("write index");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = Signature::now("test", "test@test.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .expect("commit");
}

// =======================================================================
// Git-repo mode tests
// =======================================================================

#[tokio::test]
async fn git_repo_init_detects_mode() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let info = svc.init(tmp.path().to_str().unwrap()).await.unwrap();

    assert_eq!(info.mode, SnapshotMode::GitRepo);
    assert!(info.branch.is_some());
}

#[tokio::test]
async fn git_repo_init_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    let info1 = svc.init(ws).await.unwrap();
    let info2 = svc.init(ws).await.unwrap();

    assert_eq!(info1.mode, info2.mode);
    assert_eq!(info1.branch, info2.branch);
}

#[tokio::test]
async fn git_repo_compare_clean_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "hello");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();
    let result = svc.compare(ws).await.unwrap();

    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_compare_unstaged_modify() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify tracked file
    std::fs::write(tmp.path().join("a.txt"), "modified").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "a.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Modify);
}

#[tokio::test]
async fn git_repo_compare_unstaged_create() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Create new untracked file
    std::fs::write(tmp.path().join("new.txt"), "new content").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "new.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Create);
}

#[tokio::test]
async fn git_repo_compare_unstaged_delete() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Delete tracked file
    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "a.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Delete);
}

#[tokio::test]
async fn git_repo_compare_staged_changes() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Create a file and stage it
    std::fs::write(tmp.path().join("staged.txt"), "staged").unwrap();
    let repo = Repository::open(tmp.path()).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("staged.txt")).unwrap();
    index.write().unwrap();
    drop(repo);

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);
    assert_eq!(result.staged[0].relative_path, "staged.txt");
    assert_eq!(result.staged[0].operation, FileChangeOperation::Create);
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_compare_mixed_staged_and_unstaged() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Stage a modification
    std::fs::write(tmp.path().join("a.txt"), "staged").unwrap();
    let repo = Repository::open(tmp.path()).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("a.txt")).unwrap();
    index.write().unwrap();
    drop(repo);

    // Modify again (unstaged on top of staged)
    std::fs::write(tmp.path().join("a.txt"), "unstaged on top").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);
    assert_eq!(result.staged[0].operation, FileChangeOperation::Modify);
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Modify);
}

#[tokio::test]
async fn git_repo_baseline_content_tracked_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "readme.txt", "Hello from baseline");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    let content = svc.get_baseline_content(ws, "readme.txt").await.unwrap();
    assert_eq!(content.as_deref(), Some("Hello from baseline"));
}

#[tokio::test]
async fn git_repo_baseline_content_untracked_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "tracked");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Create a new file that isn't committed
    std::fs::write(tmp.path().join("new.txt"), "new").unwrap();

    let content = svc.get_baseline_content(ws, "new.txt").await.unwrap();
    assert!(content.is_none());
}

#[tokio::test]
async fn git_repo_baseline_content_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    let content = svc.get_baseline_content(ws, "nonexistent.txt").await.unwrap();
    assert!(content.is_none());
}

// =======================================================================
// Snapshot mode tests
// =======================================================================

#[tokio::test]
async fn snapshot_init_detects_mode() {
    let tmp = tempfile::tempdir().unwrap();
    // Plain directory, no .git
    std::fs::write(tmp.path().join("hello.txt"), "hello").unwrap();

    let svc = SnapshotService::new();
    let info = svc.init(tmp.path().to_str().unwrap()).await.unwrap();

    assert_eq!(info.mode, SnapshotMode::Snapshot);
    assert!(info.branch.is_none());
}

#[tokio::test]
async fn snapshot_init_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "data").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    let info1 = svc.init(ws).await.unwrap();
    let info2 = svc.init(ws).await.unwrap();

    assert_eq!(info1.mode, info2.mode);
}

#[tokio::test]
async fn snapshot_compare_clean_after_init() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), "hello").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Right after init, the baseline matches the workspace — no changes
    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn snapshot_compare_detects_new_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("original.txt"), "original").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Add a new file after baseline
    std::fs::write(tmp.path().join("added.txt"), "new content").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "added.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Create);
}

#[tokio::test]
async fn snapshot_compare_detects_modified_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("data.txt"), "original").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify after baseline
    std::fs::write(tmp.path().join("data.txt"), "modified content").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "data.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Modify);
}

#[tokio::test]
async fn snapshot_compare_detects_deleted_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("to_delete.txt"), "content").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Delete after baseline
    std::fs::remove_file(tmp.path().join("to_delete.txt")).unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].relative_path, "to_delete.txt");
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Delete);
}

#[tokio::test]
async fn snapshot_baseline_content_returns_initial_content() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("doc.txt"), "initial content").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify the file
    std::fs::write(tmp.path().join("doc.txt"), "changed content").unwrap();

    // Baseline should still return the initial version
    let content = svc.get_baseline_content(ws, "doc.txt").await.unwrap();
    assert_eq!(content.as_deref(), Some("initial content"));
}

#[tokio::test]
async fn snapshot_baseline_content_new_file_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("existing.txt"), "exists").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Add a new file after baseline
    std::fs::write(tmp.path().join("new.txt"), "new").unwrap();

    let content = svc.get_baseline_content(ws, "new.txt").await.unwrap();
    assert!(content.is_none());
}

#[tokio::test]
async fn snapshot_excludes_node_modules() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("app.js"), "console.log('hi')").unwrap();
    std::fs::create_dir(tmp.path().join("node_modules")).unwrap();
    std::fs::write(tmp.path().join("node_modules/dep.js"), "module.exports = {}").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // After init, workspace is clean (node_modules excluded from tracking)
    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());

    // Adding a file to node_modules should not show up as a change
    std::fs::write(tmp.path().join("node_modules/new_dep.js"), "new dep").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

// =======================================================================
// Error cases
// =======================================================================

#[tokio::test]
async fn init_nonexistent_workspace_errors() {
    let svc = SnapshotService::new();
    let result = svc.init("/nonexistent/path/xyz123abc").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn compare_without_init_errors() {
    let svc = SnapshotService::new();
    let result = svc.compare("/some/workspace").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn baseline_without_init_errors() {
    let svc = SnapshotService::new();
    let result = svc.get_baseline_content("/some/ws", "file.txt").await;
    assert!(result.is_err());
}

// =======================================================================
// Full path validation
// =======================================================================

#[tokio::test]
async fn compare_result_contains_full_paths() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "src/main.rs", "fn main() {}");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify the file
    std::fs::write(tmp.path().join("src/main.rs"), "fn main() { println!(\"hi\") }").unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.unstaged.len(), 1);

    // relative_path should be just the file path relative to workspace
    assert_eq!(result.unstaged[0].relative_path, "src/main.rs");

    // file_path should contain the workspace prefix
    let canonical = std::fs::canonicalize(tmp.path()).unwrap();
    assert!(result.unstaged[0].file_path.starts_with(canonical.to_str().unwrap()));
}

// =======================================================================
// Stage / unstage tests (git-repo mode)
// =======================================================================

#[tokio::test]
async fn git_repo_stage_file_moves_to_staged() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify and stage
    std::fs::write(tmp.path().join("a.txt"), "modified").unwrap();
    svc.stage_file(ws, "a.txt").await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);
    assert_eq!(result.staged[0].relative_path, "a.txt");
    assert_eq!(result.staged[0].operation, FileChangeOperation::Modify);
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_stage_file_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();
    svc.stage_file(ws, "a.txt").await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);
    assert_eq!(result.staged[0].operation, FileChangeOperation::Delete);
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_stage_all_includes_deletions() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "a");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Create b.txt, modify a.txt, then stage all
    std::fs::write(tmp.path().join("b.txt"), "b").unwrap();
    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();
    svc.stage_all(ws).await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 2);
    assert!(result.unstaged.is_empty());

    let del = result.staged.iter().find(|e| e.relative_path == "a.txt").unwrap();
    assert_eq!(del.operation, FileChangeOperation::Delete);

    let add = result.staged.iter().find(|e| e.relative_path == "b.txt").unwrap();
    assert_eq!(add.operation, FileChangeOperation::Create);
}

#[tokio::test]
async fn git_repo_unstage_file_moves_back() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::write(tmp.path().join("a.txt"), "modified").unwrap();
    svc.stage_file(ws, "a.txt").await.unwrap();

    // Confirm staged
    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);

    // Unstage
    svc.unstage_file(ws, "a.txt").await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);
    assert_eq!(result.unstaged[0].operation, FileChangeOperation::Modify);
}

#[tokio::test]
async fn git_repo_unstage_all() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "a");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::write(tmp.path().join("a.txt"), "changed").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "new").unwrap();
    svc.stage_all(ws).await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 2);

    svc.unstage_all(ws).await.unwrap();

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 2);
}

// =======================================================================
// Discard tests (git-repo mode)
// =======================================================================

#[tokio::test]
async fn git_repo_discard_modified_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::write(tmp.path().join("a.txt"), "modified").unwrap();

    svc.discard_file(ws, "a.txt", FileChangeOperation::Modify)
        .await
        .unwrap();

    // File restored to baseline
    let content = std::fs::read_to_string(tmp.path().join("a.txt")).unwrap();
    assert_eq!(content, "original");

    // Workspace is clean
    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_discard_created_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "a");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::write(tmp.path().join("new.txt"), "new content").unwrap();

    svc.discard_file(ws, "new.txt", FileChangeOperation::Create)
        .await
        .unwrap();

    // File deleted
    assert!(!tmp.path().join("new.txt").exists());

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_discard_deleted_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();

    svc.discard_file(ws, "a.txt", FileChangeOperation::Delete)
        .await
        .unwrap();

    // File restored
    assert!(tmp.path().join("a.txt").exists());
    let content = std::fs::read_to_string(tmp.path().join("a.txt")).unwrap();
    assert_eq!(content, "content");

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

// =======================================================================
// Reset tests (git-repo mode)
// =======================================================================

#[tokio::test]
async fn git_repo_reset_staged_modified_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "original");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify and stage
    std::fs::write(tmp.path().join("a.txt"), "modified").unwrap();
    svc.stage_file(ws, "a.txt").await.unwrap();

    // Reset: should unstage AND restore
    svc.reset_file(ws, "a.txt", FileChangeOperation::Modify).await.unwrap();

    let content = std::fs::read_to_string(tmp.path().join("a.txt")).unwrap();
    assert_eq!(content, "original");

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_reset_staged_created_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Create and stage
    std::fs::write(tmp.path().join("new.txt"), "new").unwrap();
    svc.stage_file(ws, "new.txt").await.unwrap();

    // Reset: should unstage AND delete
    svc.reset_file(ws, "new.txt", FileChangeOperation::Create)
        .await
        .unwrap();

    assert!(!tmp.path().join("new.txt").exists());

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

#[tokio::test]
async fn git_repo_reset_staged_deleted_file() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Delete and stage
    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();
    svc.stage_file(ws, "a.txt").await.unwrap();

    // Reset: should unstage AND restore
    svc.reset_file(ws, "a.txt", FileChangeOperation::Delete).await.unwrap();

    assert!(tmp.path().join("a.txt").exists());
    let content = std::fs::read_to_string(tmp.path().join("a.txt")).unwrap();
    assert_eq!(content, "content");

    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert!(result.unstaged.is_empty());
}

// =======================================================================
// Dispose tests
// =======================================================================

#[tokio::test]
async fn snapshot_dispose_cleans_up_temp_repo() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "data").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Verify the workspace is still usable before dispose
    assert!(svc.compare(ws).await.is_ok());

    svc.dispose(ws).await.unwrap();

    // After dispose, the workspace is no longer tracked
    assert!(svc.compare(ws).await.is_err());
}

#[tokio::test]
async fn git_repo_dispose_does_not_delete_dot_git() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "a.txt", "content");

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    svc.dispose(ws).await.unwrap();

    // .git directory should still exist
    assert!(tmp.path().join(".git").exists());

    // But workspace is no longer tracked
    assert!(svc.compare(ws).await.is_err());
}

#[tokio::test]
async fn dispose_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "data").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    svc.dispose(ws).await.unwrap();
    // Second dispose should not error
    svc.dispose(ws).await.unwrap();
}

#[tokio::test]
async fn dispose_without_init_is_ok() {
    let svc = SnapshotService::new();
    // Dispose on never-initialized workspace should be fine
    svc.dispose("/some/nonexistent/path").await.unwrap();
}

// =======================================================================
// Full stage -> compare -> unstage -> discard flow
// =======================================================================

#[tokio::test]
async fn single_file_operations_use_literal_paths() {
    let names = [
        ("file[1].txt", "file1.txt"),
        ("dir[1]/a.txt", "dir1/a.txt"),
        ("!file.txt", "file.txt"),
        #[cfg(unix)]
        ("file*.txt", "file1.txt"),
        #[cfg(unix)]
        ("file?.txt", "file1.txt"),
        #[cfg(unix)]
        (r"file\1.txt", "file1.txt"),
    ];
    for git_repo in [true, false] {
        for (literal, neighbor) in names {
            let tmp = tempfile::tempdir().unwrap();
            if git_repo {
                init_repo_with_files(tmp.path(), &[(literal, "original"), (neighbor, "original")]);
            } else {
                for name in [literal, neighbor] {
                    let path = tmp.path().join(name);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, "original").unwrap();
                }
            }
            let svc = SnapshotService::new();
            let ws = tmp.path().to_str().unwrap();
            svc.init(ws).await.unwrap();
            let repo = Repository::open(svc.repo_path_for(ws).unwrap()).unwrap();
            let indexed = |name: &str| {
                let mut index = repo.index().unwrap();
                index.read(true).unwrap();
                index.get_path(Path::new(name), 0).map(|entry| (entry.id, entry.mode))
            };
            let baseline = indexed(literal).unwrap();
            let neighbor_baseline = indexed(neighbor).unwrap();
            std::fs::write(tmp.path().join(literal), "staged").unwrap();
            std::fs::write(tmp.path().join(neighbor), "staged neighbor").unwrap();
            svc.stage_file(ws, literal).await.unwrap();
            assert_ne!(indexed(literal), Some(baseline));
            assert_eq!(indexed(neighbor), Some(neighbor_baseline));

            svc.stage_file(ws, neighbor).await.unwrap();
            let neighbor_staged = indexed(neighbor).unwrap();
            std::fs::write(tmp.path().join(neighbor), "unstaged neighbor").unwrap();
            let assert_neighbor = || {
                assert_eq!(indexed(neighbor), Some(neighbor_staged), "index changed for {neighbor} via {literal}");
                assert_eq!(std::fs::read(tmp.path().join(neighbor)).unwrap(), b"unstaged neighbor");
            };

            svc.unstage_file(ws, literal).await.unwrap();
            assert_eq!(indexed(literal), Some(baseline));
            assert_eq!(std::fs::read(tmp.path().join(literal)).unwrap(), b"staged");
            assert_neighbor();
            svc.discard_file(ws, literal, FileChangeOperation::Modify).await.unwrap();
            assert_eq!(std::fs::read(tmp.path().join(literal)).unwrap(), b"original");
            assert_neighbor();

            for operation in [FileChangeOperation::Modify, FileChangeOperation::Delete] {
                if operation == FileChangeOperation::Modify {
                    std::fs::write(tmp.path().join(literal), "modified").unwrap();
                } else {
                    std::fs::remove_file(tmp.path().join(literal)).unwrap();
                }
                svc.stage_file(ws, literal).await.unwrap();
                assert_ne!(indexed(literal), Some(baseline));
                assert_neighbor();
                svc.reset_file(ws, literal, operation).await.unwrap();
                assert_eq!(indexed(literal), Some(baseline));
                assert_eq!(std::fs::read(tmp.path().join(literal)).unwrap(), b"original");
                assert_neighbor();
            }
            drop(repo);
            svc.dispose(ws).await.unwrap();
        }
    }
}

#[tokio::test]
async fn single_file_missing_literal_and_directory_do_not_checkout_neighbors() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_files(tmp.path(), &[("file1.txt", "original"), ("dir/a.txt", "original")]);
    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();
    for name in ["file1.txt", "dir/a.txt"] {
        std::fs::write(tmp.path().join(name), "staged").unwrap();
        svc.stage_file(ws, name).await.unwrap();
        std::fs::write(tmp.path().join(name), "unstaged").unwrap();
    }
    let repo = Repository::open(tmp.path()).unwrap();
    let index_before = std::fs::read(repo.path().join("index")).unwrap();
    for name in ["file[1].txt", "dir"] {
        assert!(svc.discard_file(ws, name, FileChangeOperation::Modify).await.is_err());
    }
    assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), index_before);

    // A new literal file is absent in HEAD: unstage removes only that entry,
    // while reset(Create) additionally removes only that working file.
    std::fs::write(tmp.path().join("file[1].txt"), "new").unwrap();
    svc.stage_file(ws, "file[1].txt").await.unwrap();
    svc.unstage_file(ws, "file[1].txt").await.unwrap();
    svc.stage_file(ws, "file[1].txt").await.unwrap();
    svc.reset_file(ws, "file[1].txt", FileChangeOperation::Create).await.unwrap();
    assert!(!tmp.path().join("file[1].txt").exists());
    let mut index = repo.index().unwrap();
    index.read(true).unwrap();
    assert!(index.get_path(Path::new("file[1].txt"), 0).is_none());
    for name in ["file1.txt", "dir/a.txt"] {
        let entry = index.get_path(Path::new(name), 0).unwrap();
        assert_eq!(repo.find_blob(entry.id).unwrap().content(), b"staged");
        assert_eq!(std::fs::read(tmp.path().join(name)).unwrap(), b"unstaged");
    }
}

#[tokio::test]
async fn single_file_unstage_preserves_modes_and_neighbor_conflicts() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_files(tmp.path(), &[("file[1].txt", "original"), ("file1.txt", "original")]);
    let repo = Repository::open(tmp.path()).unwrap();
    // Exercise the index's case-folding contract, even on a Unix filesystem.
    repo.config().unwrap().set_bool("core.ignorecase", true).unwrap();
    let mut index = repo.index().unwrap();
    let baseline = index.get_path(Path::new("file[1].txt"), 0).unwrap();
    let conflict_id = repo.blob(b"conflict").unwrap();
    for name in ["file[1].txt", "file1.txt"] {
        let mut entry = index.get_path(Path::new(name), 0).unwrap();
        index.remove_path(Path::new(name)).unwrap();
        entry.id = conflict_id;
        entry.mode = 0o100755;
        for stage in 1..=3 {
            entry.flags = (entry.flags & !0x3000) | (stage << 12);
            index.add(&entry).unwrap();
        }
    }
    index.write().unwrap();
    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();
    svc.unstage_file(ws, "FILE[1].TXT").await.unwrap();
    index.read(true).unwrap();
    let restored = index.get_path(Path::new("file[1].txt"), 0).unwrap();
    assert_eq!((restored.id, restored.mode), (baseline.id, baseline.mode));
    for stage in 1..=3 {
        assert!(index.get_path(Path::new("file[1].txt"), stage).is_none());
        let neighbor = index.get_path(Path::new("file1.txt"), stage).unwrap();
        assert_eq!((neighbor.id, neighbor.mode), (conflict_id, 0o100755));
    }
    assert!(index.get_path(Path::new("file1.txt"), 0).is_none());
}

#[tokio::test]
async fn single_file_reset_stops_on_index_or_head_error() {
    for operation in [FileChangeOperation::Create, FileChangeOperation::Modify, FileChangeOperation::Delete] {
        let tmp = tempfile::tempdir().unwrap();
        if operation == FileChangeOperation::Create {
            init_empty_repo(tmp.path());
        } else {
            init_repo_with_file(tmp.path(), "a.txt", "original");
        }
        let svc = SnapshotService::new();
        let ws = tmp.path().to_str().unwrap();
        svc.init(ws).await.unwrap();
        if operation == FileChangeOperation::Delete {
            std::fs::remove_file(tmp.path().join("a.txt")).unwrap();
        } else {
            std::fs::write(tmp.path().join("a.txt"), "staged").unwrap();
        }
        svc.stage_file(ws, "a.txt").await.unwrap();
        let repo = Repository::open(tmp.path()).unwrap();
        let index_before = std::fs::read(repo.path().join("index")).unwrap();
        std::fs::write(repo.path().join("index.lock"), "held by another writer").unwrap();
        assert!(svc.reset_file(ws, "a.txt", operation).await.is_err());
        assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), index_before);
        if operation == FileChangeOperation::Delete {
            assert!(!tmp.path().join("a.txt").exists());
        } else {
            assert_eq!(std::fs::read(tmp.path().join("a.txt")).unwrap(), b"staged");
        }
    }

    // Missing HEAD is an error, not evidence that the file is unstaged.
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repository::init(tmp.path()).unwrap();
    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();
    std::fs::write(tmp.path().join("new.txt"), "keep").unwrap();
    svc.stage_file(ws, "new.txt").await.unwrap();
    assert!(svc.reset_file(ws, "new.txt", FileChangeOperation::Create).await.is_err());
    assert_eq!(std::fs::read(tmp.path().join("new.txt")).unwrap(), b"keep");
    assert!(repo.index().unwrap().get_path(Path::new("new.txt"), 0).is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn single_file_stage_handles_dangling_links_and_metadata_errors() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_file(tmp.path(), "dir/a.txt", "original");
    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();
    std::os::unix::fs::symlink("missing", tmp.path().join("link")).unwrap();
    svc.stage_file(ws, "link").await.unwrap();
    let repo = Repository::open(tmp.path()).unwrap();
    let mut index = repo.index().unwrap();
    let entry = index.get_path(Path::new("link"), 0).expect("dangling link must be staged");
    assert_eq!(entry.mode, 0o120000);
    assert_eq!(repo.find_blob(entry.id).unwrap().content(), b"missing");

    // ELOOP is deterministic even when running as root; it is not a deletion.
    let before = index.get_path(Path::new("dir/a.txt"), 0).unwrap().id;
    std::fs::remove_file(tmp.path().join("dir/a.txt")).unwrap();
    std::fs::remove_dir(tmp.path().join("dir")).unwrap();
    std::os::unix::fs::symlink("dir", tmp.path().join("dir")).unwrap();
    assert!(svc.stage_file(ws, "dir/a.txt").await.is_err());
    index.read(true).unwrap();
    assert_eq!(index.get_path(Path::new("dir/a.txt"), 0).unwrap().id, before);
}

// =======================================================================
// Snapshot mode: stage/discard/dispose
// =======================================================================

#[tokio::test]
async fn snapshot_stage_and_discard_flow() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("doc.txt"), "initial").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();
    svc.init(ws).await.unwrap();

    // Modify the file
    std::fs::write(tmp.path().join("doc.txt"), "changed").unwrap();
    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.unstaged.len(), 1);

    // Stage all
    svc.stage_all(ws).await.unwrap();
    let result = svc.compare(ws).await.unwrap();
    assert_eq!(result.staged.len(), 1);
    assert!(result.unstaged.is_empty());

    // Unstage all
    svc.unstage_all(ws).await.unwrap();
    let result = svc.compare(ws).await.unwrap();
    assert!(result.staged.is_empty());
    assert_eq!(result.unstaged.len(), 1);

    // Discard modification
    svc.discard_file(ws, "doc.txt", FileChangeOperation::Modify)
        .await
        .unwrap();
    let content = std::fs::read_to_string(tmp.path().join("doc.txt")).unwrap();
    assert_eq!(content, "initial");
}

// =======================================================================
// Task 3: DashMap key canonicalization
// =======================================================================

#[tokio::test]
async fn snapshot_init_same_dir_two_string_forms_one_entry() {
    // A git repo so we exercise the cheap GitRepo path (the canonicalization
    // fix applies to both modes — the key is the canonical path).
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();

    // Two string forms of the same directory that canonicalize equal:
    //   1. the path as-is
    //   2. the same path with a trailing separator appended
    let base = tmp.path().to_str().unwrap().to_string();
    let with_sep = format!("{}{}", base, std::path::MAIN_SEPARATOR);

    svc.init(&base).await.unwrap();
    svc.init(&with_sep).await.unwrap();

    // Both forms point at the same canonical workspace -> exactly one entry.
    assert_eq!(
        svc.workspace_count(),
        1,
        "two string forms of the same dir must collapse to one DashMap entry"
    );
}

// =======================================================================
// Task 4: Reference counting
// =======================================================================

#[tokio::test]
async fn snapshot_refcount_disposes_only_on_last() {
    // Non-git dir -> Snapshot mode (creates a temp repo we can observe).
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("f.txt"), "data").unwrap();

    let svc = SnapshotService::new();
    let ws = tmp.path().to_str().unwrap();

    // init twice -> refcount = 2
    svc.init(ws).await.unwrap();
    svc.init(ws).await.unwrap();

    let repo_path = svc.repo_path_for(ws).expect("repo path tracked after init");
    assert!(repo_path.exists(), "snapshot temp repo should exist after init");

    // First dispose drops refcount to 1: entry + temp repo remain.
    svc.dispose(ws).await.unwrap();
    assert!(svc.is_tracked(ws), "entry must remain after first dispose (refcount 2->1)");
    assert!(repo_path.exists(), "temp repo must remain after first dispose");
    assert!(svc.compare(ws).await.is_ok(), "still usable after first dispose");

    // Second dispose drops refcount to 0: entry + temp repo removed.
    svc.dispose(ws).await.unwrap();
    assert!(!svc.is_tracked(ws), "entry must be gone after last dispose");
    assert!(!repo_path.exists(), "temp repo must be removed after last dispose");
    assert!(svc.compare(ws).await.is_err(), "no longer usable after last dispose");
}

// =======================================================================
// Task 5: SnapshotMode::Disabled { reason } wire shape
// =======================================================================

#[test]
fn disabled_mode_serializes_to_wire_disabled_with_reason() {
    use nomifun_api_types::{SnapshotInfoResponse, SnapshotMode as WireMode};

    let resp = SnapshotInfoResponse {
        mode: WireMode::Disabled,
        branch: None,
        reason: Some("workspace too large".to_string()),
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["mode"], "disabled", "disabled mode must serialize to wire \"disabled\"");
    assert_eq!(json["reason"], "workspace too large", "reason must be carried on the wire");

    // git-repo / snapshot wire values are unchanged.
    let git = SnapshotInfoResponse {
        mode: WireMode::GitRepo,
        branch: Some("main".into()),
        reason: None,
    };
    assert_eq!(serde_json::to_value(&git).unwrap()["mode"], "git-repo");
    let snap = SnapshotInfoResponse {
        mode: WireMode::Snapshot,
        branch: None,
        reason: None,
    };
    assert_eq!(serde_json::to_value(&snap).unwrap()["mode"], "snapshot");
}

// =======================================================================
// Task 6: Snapshot-branch safety guard
// =======================================================================

#[cfg(windows)]
#[tokio::test]
async fn guard_refuses_drive_root_returns_disabled() {
    let svc = SnapshotService::new();

    // C:\ is a drive root: snapshotting it would walk the whole drive.
    let info = svc.init("C:\\").await.unwrap();

    assert!(
        matches!(info.mode, SnapshotMode::Disabled { .. }),
        "drive root must be Disabled, got {:?}",
        info.mode
    );
    // No temp repo created, and the workspace is not tracked.
    assert!(!svc.is_tracked("C:\\"), "Disabled workspace must not be tracked");
}

#[tokio::test]
async fn guard_allows_normal_small_non_git_dir() {
    // A small, ordinary non-git directory must still get Snapshot mode.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "world").unwrap();

    let svc = SnapshotService::new();
    let info = svc.init(tmp.path().to_str().unwrap()).await.unwrap();

    assert_eq!(info.mode, SnapshotMode::Snapshot, "small non-git dir must be Snapshot");
}

#[tokio::test]
async fn guard_does_not_affect_git_repo_dir() {
    // A .git dir takes the GitRepo path and never consults the guard.
    let tmp = tempfile::tempdir().unwrap();
    init_empty_repo(tmp.path());

    let svc = SnapshotService::new();
    let info = svc.init(tmp.path().to_str().unwrap()).await.unwrap();

    assert_eq!(info.mode, SnapshotMode::GitRepo, ".git dir must be GitRepo (guard bypassed)");
}
