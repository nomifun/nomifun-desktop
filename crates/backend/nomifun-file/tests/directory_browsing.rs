//! Integration tests for directory browsing and file metadata (task 7.3).
//!
//! These tests exercise the full `FileService` through the `IFileService` trait,
//! including path validation, .gitignore handling, and caching behavior.

use std::fs;
use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileService, IFileService};
use nomifun_realtime::EventBroadcaster;

/// A no-op broadcaster for testing (events are silently discarded).
struct NoopBroadcaster;

impl EventBroadcaster for NoopBroadcaster {
    fn broadcast(&self, _event: WebSocketMessage<serde_json::Value>) {}
}

/// Create a `FileService` whose sandbox is rooted at the given temp directory.
fn make_service(root: &std::path::Path) -> FileService {
    FileService::new(Arc::new(NoopBroadcaster), vec![root.to_path_buf()])
}

// -----------------------------------------------------------------------
// getFilesByDir
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_files_by_dir_lists_children() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/nested.txt"), "nested").unwrap();

    let svc = make_service(dir.path());
    let root = dir.path().to_str().unwrap();

    let items = svc.get_files_by_dir(root, root).await.unwrap();

    // Should have: sub/ and a.txt
    assert_eq!(items.len(), 2);

    let sub = items.iter().find(|i| i.name == "sub").unwrap();
    assert!(sub.is_dir);
    assert_eq!(sub.children.len(), 1);
    assert_eq!(sub.children[0].name, "nested.txt");

    let file = items.iter().find(|i| i.name == "a.txt").unwrap();
    assert!(!file.is_dir);
    assert!(file.children.is_empty());
}

#[tokio::test]
async fn get_files_by_dir_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());
    let root = dir.path().to_str().unwrap();

    let items = svc.get_files_by_dir(root, root).await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn get_files_by_dir_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("src");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("main.rs"), "fn main(){}").unwrap();
    fs::write(sub.join("lib.rs"), "pub mod foo;").unwrap();

    let svc = make_service(dir.path());
    let root = dir.path().to_str().unwrap();

    let items = svc.get_files_by_dir(sub.to_str().unwrap(), root).await.unwrap();

    assert_eq!(items.len(), 2);
    // Relative paths should be relative to root, not to sub
    assert!(items[0].relative_path.starts_with("src/"));
}

#[tokio::test]
async fn get_files_by_dir_rejects_path_outside_sandbox() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "secret").unwrap();

    let svc = make_service(sandbox.path());

    let result = svc
        .get_files_by_dir(outside.path().to_str().unwrap(), outside.path().to_str().unwrap())
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_files_by_dir_nonexistent_dir() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("nonexistent");

    let svc = make_service(dir.path());

    let result = svc
        .get_files_by_dir(fake.to_str().unwrap(), dir.path().to_str().unwrap())
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_files_by_dir_directories_sorted_first() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("z_file.txt"), "").unwrap();
    fs::create_dir(dir.path().join("a_dir")).unwrap();
    fs::write(dir.path().join("a_file.txt"), "").unwrap();

    let svc = make_service(dir.path());
    let root = dir.path().to_str().unwrap();

    let items = svc.get_files_by_dir(root, root).await.unwrap();

    // Directory first
    assert!(items[0].is_dir);
    assert_eq!(items[0].name, "a_dir");
    // Then files alphabetically
    assert_eq!(items[1].name, "a_file.txt");
    assert_eq!(items[2].name, "z_file.txt");
}

// -----------------------------------------------------------------------
// listWorkspaceFiles
// -----------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_files_recursive() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("root.txt"), "").unwrap();
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::write(dir.path().join("a/nested.txt"), "").unwrap();
    fs::create_dir(dir.path().join("a/b")).unwrap();
    fs::write(dir.path().join("a/b/deep.txt"), "").unwrap();

    let svc = make_service(dir.path());
    let files = svc.list_workspace_files(dir.path().to_str().unwrap()).await.unwrap();

    assert_eq!(files.len(), 3);
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"root.txt"));
    assert!(names.contains(&"nested.txt"));
    assert!(names.contains(&"deep.txt"));
}

#[tokio::test]
async fn list_workspace_files_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".gitignore"), "*.log\nbuild/\n").unwrap();
    fs::write(dir.path().join("app.rs"), "").unwrap();
    fs::write(dir.path().join("debug.log"), "").unwrap();
    fs::create_dir(dir.path().join("build")).unwrap();
    fs::write(dir.path().join("build/output.js"), "").unwrap();

    let svc = make_service(dir.path());
    let files = svc.list_workspace_files(dir.path().to_str().unwrap()).await.unwrap();

    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"app.rs"));
    assert!(names.contains(&".gitignore"));
    assert!(!names.contains(&"debug.log"));
    assert!(!names.contains(&"output.js"));
}

#[tokio::test]
async fn list_workspace_files_empty_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let files = svc.list_workspace_files(dir.path().to_str().unwrap()).await.unwrap();

    assert!(files.is_empty());
}

#[tokio::test]
async fn list_workspace_files_rejects_outside_sandbox() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    let svc = make_service(sandbox.path());

    let result = svc.list_workspace_files(outside.path().to_str().unwrap()).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn list_workspace_files_cache_hit() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), "data").unwrap();

    let svc = make_service(dir.path());
    let root = dir.path().to_str().unwrap();

    // First call populates cache
    let first = svc.list_workspace_files(root).await.unwrap();
    assert_eq!(first.len(), 1);

    // Add a file — should NOT appear due to cache
    fs::write(dir.path().join("new.txt"), "new").unwrap();
    let second = svc.list_workspace_files(root).await.unwrap();
    assert_eq!(second.len(), 1); // Still cached

    // Invalidate cache
    svc.invalidate_cache(&std::fs::canonicalize(dir.path()).unwrap().to_string_lossy());

    // Now should see new file
    let third = svc.list_workspace_files(root).await.unwrap();
    assert_eq!(third.len(), 2);
}

#[tokio::test]
async fn list_workspace_files_relative_paths() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/utils")).unwrap();
    fs::write(dir.path().join("src/utils/helper.ts"), "").unwrap();

    let svc = make_service(dir.path());
    let files = svc.list_workspace_files(dir.path().to_str().unwrap()).await.unwrap();

    let helper = files.iter().find(|f| f.name == "helper.ts").unwrap();
    assert_eq!(helper.relative_path, "src/utils/helper.ts");
}

#[cfg(unix)]
#[tokio::test]
async fn list_workspace_files_skips_directory_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("builtin-skills/auto-inject/nomifun-skills");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "---\ndescription: test\n---\nbody").unwrap();

    let workspace_skills_dir = dir.path().join("workspace/.claude/skills");
    fs::create_dir_all(&workspace_skills_dir).unwrap();
    std::os::unix::fs::symlink(&skill_dir, workspace_skills_dir.join("nomifun-skills")).unwrap();

    let svc = make_service(dir.path().join("workspace").as_path());
    let files = svc
        .list_workspace_files(dir.path().join("workspace").to_str().unwrap())
        .await
        .unwrap();

    assert!(
        files.iter().all(|file| file.name != "nomifun-skills"),
        "directory symlink should not be surfaced as a file: {files:?}"
    );
}

// -----------------------------------------------------------------------
// getFileMetadata
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_file_metadata_text_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    fs::write(&file, "hello world").unwrap();

    let svc = make_service(dir.path());
    let meta = svc.get_file_metadata(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(meta.name, "hello.txt");
    assert_eq!(meta.size, 11);
    assert_eq!(meta.mime_type, "text/plain");
    assert!(!meta.is_directory);
    assert!(meta.last_modified > 0);
}

#[tokio::test]
async fn get_file_metadata_image() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("photo.png");
    fs::write(&png, [0x89, 0x50, 0x4E, 0x47]).unwrap();

    let svc = make_service(dir.path());
    let meta = svc.get_file_metadata(png.to_str().unwrap(), None).await.unwrap();

    assert_eq!(meta.mime_type, "image/png");
}

#[tokio::test]
async fn get_file_metadata_directory() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("mydir");
    fs::create_dir(&sub).unwrap();

    let svc = make_service(dir.path());
    let meta = svc.get_file_metadata(sub.to_str().unwrap(), None).await.unwrap();

    assert!(meta.is_directory);
    assert_eq!(meta.mime_type, "inode/directory");
    assert_eq!(meta.name, "mydir");
}

#[tokio::test]
async fn get_file_metadata_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("missing.txt");

    let svc = make_service(dir.path());
    let result = svc.get_file_metadata(fake.to_str().unwrap(), None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_file_metadata_outside_sandbox() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    fs::write(&secret, "secret").unwrap();

    let svc = make_service(sandbox.path());
    let result = svc.get_file_metadata(secret.to_str().unwrap(), None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_file_metadata_json_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.json");
    fs::write(&file, r#"{"key":"value"}"#).unwrap();

    let svc = make_service(dir.path());
    let meta = svc.get_file_metadata(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(meta.mime_type, "application/json");
}
