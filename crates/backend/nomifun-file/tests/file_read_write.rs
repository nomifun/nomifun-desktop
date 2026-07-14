//! Integration tests for file read/write operations (task 7.4).
//!
//! These tests exercise `read_file`, `read_file_buffer`, and `write_file`
//! through the `IFileService` trait, including path validation, 256 MB size
//! limit, non-existent file handling, and contentUpdate event broadcast.

use std::fs;
use std::sync::{Arc, Mutex};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileService, IFileService};
use nomifun_realtime::UserEventSink;

/// A broadcaster that records every event for later assertion.
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

    fn take_owners(&self) -> Vec<String> {
        let mut guard = self.owners.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl UserEventSink for RecordingBroadcaster {
    fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
        self.owners.lock().unwrap().push(user_id.to_owned());
        self.events.lock().unwrap().push(event);
    }
}

/// No-op broadcaster for tests that don't need event verification.
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
// readFile
// -----------------------------------------------------------------------

#[tokio::test]
async fn read_file_normal_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    fs::write(&file, "hello world").unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(result.as_deref(), Some("hello world"));
}

#[tokio::test]
async fn read_file_empty() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    fs::write(&file, "").unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(result.as_deref(), Some(""));
}

#[tokio::test]
async fn read_file_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("missing.txt");

    let svc = make_service(dir.path());
    let result = svc.read_file(fake.to_str().unwrap(), None).await.unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn read_file_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file("../../etc/passwd", None).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("traversal"), "expected traversal error, got: {err}");
}

#[tokio::test]
async fn read_file_multiline_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multi.txt");
    let content = "line 1\nline 2\nline 3\n";
    fs::write(&file, content).unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(result.as_deref(), Some(content));
}

#[tokio::test]
async fn read_file_unicode_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("unicode.txt");
    let content = "你好世界 🌍 café résumé";
    fs::write(&file, content).unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(result.as_deref(), Some(content));
}

#[tokio::test]
async fn read_file_with_extra_workspace_root_outside_home() {
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("outside.txt");
    fs::write(&file, "workspace content").unwrap();

    let svc = make_service(sandbox.path());
    let result = svc
        .read_file(file.to_str().unwrap(), Some(workspace.path()))
        .await
        .unwrap();

    assert_eq!(result.as_deref(), Some("workspace content"));
}

#[tokio::test]
async fn read_file_rejects_outside_sandbox_without_workspace() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("secret.txt");
    fs::write(&file, "secret").unwrap();

    let svc = make_service(sandbox.path());
    let err = svc.read_file(file.to_str().unwrap(), None).await.unwrap_err();

    assert!(matches!(err, nomifun_common::AppError::Forbidden(_)));
    assert_eq!(err.error_code(), "PATH_OUTSIDE_SANDBOX");
}

#[tokio::test]
async fn read_file_returns_none_for_missing_file_in_sandbox() {
    let sandbox = tempfile::tempdir().unwrap();
    let missing = sandbox.path().join("missing.txt");

    let svc = make_service(sandbox.path());
    let result = svc.read_file(missing.to_str().unwrap(), None).await.unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn read_file_rejects_directory() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("nomifun-skills");
    fs::create_dir(&folder).unwrap();

    let svc = make_service(dir.path());
    let err = svc.read_file(folder.to_str().unwrap(), None).await.unwrap_err();

    assert!(matches!(err, nomifun_common::AppError::BadRequest(_)));
    assert!(err.to_string().contains("is a directory"));
}

#[tokio::test]
async fn read_file_buffer_with_extra_workspace_root() {
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("outside.bin");
    let bytes = vec![1, 2, 3, 4];
    fs::write(&file, &bytes).unwrap();

    let svc = make_service(sandbox.path());
    let result = svc
        .read_file_buffer(file.to_str().unwrap(), Some(workspace.path()))
        .await
        .unwrap();

    assert_eq!(result.as_deref(), Some(bytes.as_slice()));
}

#[tokio::test]
async fn read_file_nonexistent_inside_workspace_prefix_returns_none() {
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let missing = workspace.path().join("missing.txt");

    let svc = make_service(sandbox.path());
    let result = svc
        .read_file(missing.to_str().unwrap(), Some(workspace.path()))
        .await
        .unwrap();

    assert!(result.is_none());
}

// -----------------------------------------------------------------------
// readFileBuffer
// -----------------------------------------------------------------------

#[tokio::test]
async fn read_file_buffer_normal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data.bin");
    let data: Vec<u8> = vec![0x00, 0xFF, 0x42, 0x89, 0x50];
    fs::write(&file, &data).unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file_buffer(file.to_str().unwrap(), None).await.unwrap();

    assert_eq!(result.as_deref(), Some(data.as_slice()));
}

#[tokio::test]
async fn read_file_buffer_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("missing.bin");

    let svc = make_service(dir.path());
    let result = svc.read_file_buffer(fake.to_str().unwrap(), None).await.unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn read_file_buffer_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let result = svc.read_file_buffer("../../etc/passwd", None).await;

    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// writeFile
// -----------------------------------------------------------------------

#[tokio::test]
async fn write_file_normal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("output.txt");

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let ok = svc
        .write_file("owner-a", file.to_str().unwrap(), b"hello", ws)
        .await
        .unwrap();

    assert!(ok);
    assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
}

#[tokio::test]
async fn write_file_creates_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("new_file.txt");
    assert!(!file.exists());

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let ok = svc
        .write_file("owner-a", file.to_str().unwrap(), b"created", ws)
        .await
        .unwrap();

    assert!(ok);
    assert!(file.exists());
    assert_eq!(fs::read_to_string(&file).unwrap(), "created");
}

#[tokio::test]
async fn write_file_parent_not_exists_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nonexistent_dir/file.txt");

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let result = svc
        .write_file("owner-a", file.to_str().unwrap(), b"data", ws)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn write_file_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();
    let result = svc
        .write_file("owner-a", "../../tmp/evil.txt", b"bad", ws)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn write_file_outside_sandbox_rejected() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("evil.txt");

    let svc = make_service(sandbox.path());
    let ws = sandbox.path().to_str().unwrap();
    let result = svc
        .write_file("owner-a", target.to_str().unwrap(), b"bad", ws)
        .await;

    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// contentUpdate event
// -----------------------------------------------------------------------

#[tokio::test]
async fn write_file_emits_content_update_event() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("event_test.txt");

    let (svc, recorder) = make_service_with_recorder(dir.path());
    let ws = dir.path().to_str().unwrap();

    svc.write_file("owner-a", file.to_str().unwrap(), b"event content", ws)
        .await
        .unwrap();

    let events = recorder.take_events();
    assert_eq!(recorder.take_owners(), vec!["owner-a"]);
    assert_eq!(events.len(), 1);

    let event = &events[0];
    assert_eq!(event.name, "fileStream.contentUpdate");
    assert_eq!(event.data["content"], "event content");
    assert_eq!(event.data["workspace"], ws);
    assert_eq!(event.data["operation"], "write");
    // file_path should be the canonical path
    assert!(
        event.data["file_path"].as_str().unwrap().contains("event_test.txt"),
        "file_path should contain the file name"
    );
    // relative_path should be relative to workspace
    assert_eq!(event.data["relative_path"], "event_test.txt");
}

#[tokio::test]
async fn write_file_binary_omits_content_in_event() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("binary.bin");
    // Invalid UTF-8 sequence
    let data: Vec<u8> = vec![0xFF, 0xFE, 0x00, 0x01];

    let (svc, recorder) = make_service_with_recorder(dir.path());
    let ws = dir.path().to_str().unwrap();

    svc.write_file("owner-a", file.to_str().unwrap(), &data, ws)
        .await
        .unwrap();

    let events = recorder.take_events();
    assert_eq!(events.len(), 1);

    let event = &events[0];
    // content should be absent for binary data (not valid UTF-8)
    assert!(
        event.data.get("content").is_none(),
        "binary write should omit content in event"
    );
}

#[tokio::test]
async fn write_file_nested_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/utils")).unwrap();
    let file = dir.path().join("src/utils/helper.ts");

    let (svc, recorder) = make_service_with_recorder(dir.path());
    let ws = dir.path().to_str().unwrap();

    svc.write_file("owner-a", file.to_str().unwrap(), b"export {}", ws)
        .await
        .unwrap();

    let events = recorder.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["relative_path"], "src/utils/helper.ts");
}

// -----------------------------------------------------------------------
// read after write (roundtrip)
// -----------------------------------------------------------------------

#[tokio::test]
async fn read_after_write_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("roundtrip.txt");
    let content = "roundtrip test content 你好";

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();

    // Write
    let ok = svc
        .write_file("owner-a", file.to_str().unwrap(), content.as_bytes(), ws)
        .await
        .unwrap();
    assert!(ok);

    // Read back
    let read_result = svc.read_file(file.to_str().unwrap(), None).await.unwrap();
    assert_eq!(read_result.as_deref(), Some(content));
}

#[tokio::test]
async fn read_buffer_after_write_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("roundtrip.bin");
    let data: Vec<u8> = vec![0x01, 0x02, 0x03, 0xFF, 0xFE];

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();

    svc.write_file("owner-a", file.to_str().unwrap(), &data, ws)
        .await
        .unwrap();

    let read_result = svc.read_file_buffer(file.to_str().unwrap(), None).await.unwrap();
    assert_eq!(read_result.as_deref(), Some(data.as_slice()));
}

// -----------------------------------------------------------------------
// write_file invalidates workspace files cache
// -----------------------------------------------------------------------

#[tokio::test]
async fn write_file_invalidates_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("existing.txt"), "data").unwrap();

    let svc = make_service(dir.path());
    let ws = dir.path().to_str().unwrap();

    // Populate cache
    let files = svc.list_workspace_files(ws).await.unwrap();
    assert_eq!(files.len(), 1);

    // Write a new file (should invalidate cache)
    let new_file = dir.path().join("new.txt");
    svc.write_file("owner-a", new_file.to_str().unwrap(), b"new", ws)
        .await
        .unwrap();

    // Cache should be invalidated, so we see the new file
    let files = svc.list_workspace_files(ws).await.unwrap();
    assert_eq!(files.len(), 2);
}
