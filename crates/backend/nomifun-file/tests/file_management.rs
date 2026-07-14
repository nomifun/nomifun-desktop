//! Integration tests for file management operations (task 7.5).
//!
//! Covers `copy_files_to_workspace`, `remove_entry`, `rename_entry`, and
//! `create_temp_file` through the `IFileService` trait, including path
//! validation, event broadcast, and cache invalidation.

use std::fs;
use std::sync::{Arc, Mutex};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileService, IFileService};
use nomifun_realtime::UserEventSink;

// -----------------------------------------------------------------------
// Test helpers (shared with file_read_write.rs pattern)
// -----------------------------------------------------------------------

struct RecordingBroadcaster {
    events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    owners: Mutex<Vec<String>>,
}

impl RecordingBroadcaster {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            owners: Mutex::new(Vec::new()),
        }
    }

    fn take_events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
        let mut guard = self.events.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl UserEventSink for RecordingBroadcaster {
    fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
        self.owners.lock().unwrap().push(user_id.to_owned());
        self.events.lock().unwrap().push(event);
    }
}

struct NoopBroadcaster;

impl UserEventSink for NoopBroadcaster {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
}

fn make_service(root: &std::path::Path) -> FileService {
    FileService::new(Arc::new(NoopBroadcaster), vec![root.to_path_buf()])
}

fn make_service_with_recorder(root: &std::path::Path) -> (FileService, Arc<RecordingBroadcaster>) {
    let recorder = Arc::new(RecordingBroadcaster::new());
    let svc = FileService::new(recorder.clone(), vec![root.to_path_buf()]);
    (svc, recorder)
}

// -----------------------------------------------------------------------
// copyFilesToWorkspace
// -----------------------------------------------------------------------

#[tokio::test]
async fn copy_files_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let ws_dir = dir.path().join("ws");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::write(src_dir.join("a.txt"), "hello").unwrap();

    let svc = make_service(dir.path());
    let paths = vec![src_dir.join("a.txt").to_string_lossy().into_owned()];
    let result = svc
        .copy_files_to_workspace(&paths, ws_dir.to_str().unwrap(), None)
        .await
        .unwrap();

    assert_eq!(result.copied_files.len(), 1);
    assert!(result.failed_files.is_empty());
    // Without source_root, file should be at workspace root
    assert_eq!(fs::read_to_string(ws_dir.join("a.txt")).unwrap(), "hello");
}

#[tokio::test]
async fn copy_files_with_source_root_preserves_structure() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("project");
    let ws_dir = dir.path().join("ws");
    fs::create_dir_all(src_dir.join("utils")).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::write(src_dir.join("utils/helper.ts"), "export {}").unwrap();
    fs::write(src_dir.join("index.ts"), "import {}").unwrap();

    let svc = make_service(dir.path());
    let paths = vec![
        src_dir.join("utils/helper.ts").to_string_lossy().into_owned(),
        src_dir.join("index.ts").to_string_lossy().into_owned(),
    ];

    let result = svc
        .copy_files_to_workspace(&paths, ws_dir.to_str().unwrap(), Some(src_dir.to_str().unwrap()))
        .await
        .unwrap();

    assert_eq!(result.copied_files.len(), 2);
    assert!(result.failed_files.is_empty());
    // Directory structure preserved relative to source_root
    assert_eq!(fs::read_to_string(ws_dir.join("utils/helper.ts")).unwrap(), "export {}");
    assert_eq!(fs::read_to_string(ws_dir.join("index.ts")).unwrap(), "import {}");
}

#[tokio::test]
async fn copy_files_partial_failure() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let ws_dir = dir.path().join("ws");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&ws_dir).unwrap();
    fs::write(src_dir.join("good.txt"), "ok").unwrap();

    let svc = make_service(dir.path());
    let paths = vec![
        src_dir.join("good.txt").to_string_lossy().into_owned(),
        src_dir.join("missing.txt").to_string_lossy().into_owned(),
    ];

    let result = svc
        .copy_files_to_workspace(&paths, ws_dir.to_str().unwrap(), None)
        .await
        .unwrap();

    assert_eq!(result.copied_files.len(), 1);
    assert_eq!(result.failed_files.len(), 1);
    assert!(result.failed_files[0].contains("missing.txt"));
}

#[tokio::test]
async fn copy_files_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let ws_dir = dir.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    let svc = make_service(dir.path());
    let result = svc
        .copy_files_to_workspace(&[], ws_dir.to_str().unwrap(), None)
        .await
        .unwrap();

    assert!(result.copied_files.is_empty());
    assert!(result.failed_files.is_empty());
}

#[tokio::test]
async fn copy_files_directory_in_list_is_failed() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("subdir");
    let ws = dir.path().join("ws");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&ws).unwrap();

    let svc = make_service(dir.path());
    let paths = vec![sub.to_string_lossy().into_owned()];
    let result = svc
        .copy_files_to_workspace(&paths, ws.to_str().unwrap(), None)
        .await
        .unwrap();

    // Directories are not valid source files
    assert!(result.copied_files.is_empty());
    assert_eq!(result.failed_files.len(), 1);
}

#[tokio::test]
async fn copy_files_outside_sandbox_fails() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let ws = sandbox.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(outside.path().join("secret.txt"), "secret").unwrap();

    let svc = make_service(sandbox.path());
    let paths = vec![outside.path().join("secret.txt").to_string_lossy().into_owned()];

    let result = svc
        .copy_files_to_workspace(&paths, ws.to_str().unwrap(), None)
        .await
        .unwrap();

    assert!(result.copied_files.is_empty());
    assert_eq!(result.failed_files.len(), 1);
}

// -----------------------------------------------------------------------
// removeEntry
// -----------------------------------------------------------------------

#[tokio::test]
async fn remove_entry_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("to_delete.txt");
    fs::write(&file, "bye").unwrap();
    assert!(file.exists());

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    svc.remove_entry("owner-a", file.to_str().unwrap(), ws)
        .await
        .unwrap();

    assert!(!file.exists());
}

#[tokio::test]
async fn remove_entry_directory() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("subdir");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("inner.txt"), "data").unwrap();

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    svc.remove_entry("owner-a", sub.to_str().unwrap(), ws)
        .await
        .unwrap();

    assert!(!sub.exists());
}

#[tokio::test]
async fn remove_entry_nonexistent_errors() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ghost.txt");

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let result = svc.remove_entry("owner-a", fake.to_str().unwrap(), ws).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn remove_entry_emits_delete_event() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("event_del.txt");
    fs::write(&file, "data").unwrap();

    let (svc, recorder) = make_service_with_recorder(dir.path());
    let ws = dir.path().to_str().unwrap();
    svc.remove_entry("owner-a", file.to_str().unwrap(), ws)
        .await
        .unwrap();

    let events = recorder.take_events();
    assert_eq!(events.len(), 1);

    let event = &events[0];
    assert_eq!(event.name, "fileStream.contentUpdate");
    assert_eq!(event.data["operation"], "delete");
    assert!(event.data.get("content").is_none());
    assert_eq!(event.data["relative_path"], "event_del.txt");
    assert!(event.data["file_path"].as_str().unwrap().contains("event_del.txt"));
}

#[tokio::test]
async fn remove_entry_invalidates_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "a").unwrap();
    fs::write(dir.path().join("b.txt"), "b").unwrap();

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();

    // Populate cache
    let files = svc.list_workspace_files(ws).await.unwrap();
    assert_eq!(files.len(), 2);

    // Remove a file
    let target = dir.path().join("a.txt");
    svc.remove_entry("owner-a", target.to_str().unwrap(), ws)
        .await
        .unwrap();

    // Cache should be invalidated, so we see only 1 file
    let files = svc.list_workspace_files(ws).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "b.txt");
}

#[tokio::test]
async fn remove_entry_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let result = svc.remove_entry("owner-a", "../../etc/passwd", ws).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("traversal"), "got: {err}");
}

// -----------------------------------------------------------------------
// renameEntry
// -----------------------------------------------------------------------

#[tokio::test]
async fn rename_entry_file() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.txt");
    fs::write(&old, "data").unwrap();

    let svc = make_service(dir.path());
    let new_path = svc.rename_entry(old.to_str().unwrap(), "new.txt").await.unwrap();

    assert!(!old.exists());
    assert!(new_path.contains("new.txt"));
    assert_eq!(fs::read_to_string(dir.path().join("new.txt")).unwrap(), "data");
}

#[tokio::test]
async fn rename_entry_directory() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old_dir");
    fs::create_dir(&old).unwrap();
    fs::write(old.join("inner.txt"), "inner").unwrap();

    let svc = make_service(dir.path());
    let new_path = svc.rename_entry(old.to_str().unwrap(), "new_dir").await.unwrap();

    assert!(!old.exists());
    assert!(std::path::Path::new(&new_path).is_dir());
    assert_eq!(
        fs::read_to_string(dir.path().join("new_dir/inner.txt")).unwrap(),
        "inner"
    );
}

#[tokio::test]
async fn rename_entry_target_exists_errors() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.txt");
    let existing = dir.path().join("existing.txt");
    fs::write(&old, "old").unwrap();
    fs::write(&existing, "existing").unwrap();

    let svc = make_service(dir.path());
    let result = svc.rename_entry(old.to_str().unwrap(), "existing.txt").await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("already exists"), "got: {err}");
}

#[tokio::test]
async fn rename_entry_nonexistent_source_errors() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("missing.txt");

    let svc = make_service(dir.path());
    let result = svc.rename_entry(fake.to_str().unwrap(), "new.txt").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn rename_entry_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let result = svc.rename_entry("../../etc/passwd", "new.txt").await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("traversal"), "got: {err}");
}

#[tokio::test]
async fn rename_entry_rejects_path_separator_in_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    fs::write(&file, "data").unwrap();

    let svc = make_service(dir.path());
    let result = svc.rename_entry(file.to_str().unwrap(), "sub/new.txt").await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("path separator"), "got: {err}");
}

// -----------------------------------------------------------------------
// createTempFile
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_temp_file_normal() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let path = svc.create_temp_file("test.txt").await.unwrap();

    assert!(path.contains("test.txt"));
    assert!(std::path::Path::new(&path).exists());
}

#[tokio::test]
async fn create_temp_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let path = svc.create_temp_file("empty.txt").await.unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.is_empty());
}

#[tokio::test]
async fn create_temp_file_path_in_nomifun_dir() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let path = svc.create_temp_file("check.txt").await.unwrap();

    assert!(path.contains("nomifun"), "temp path should be under nomifun dir");
}

#[tokio::test]
async fn create_temp_file_rejects_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.create_temp_file("../../malicious.txt").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("traversal"), "expected traversal error, got: {err}");
}

#[tokio::test]
async fn create_temp_file_rejects_path_separator() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.create_temp_file("sub/file.txt").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("path separator"),
        "expected path separator error, got: {err}"
    );
}

#[tokio::test]
async fn create_temp_file_rejects_null_byte() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.create_temp_file("evil\0name.txt").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("traversal"), "expected traversal error, got: {err}");
}
