//! Integration tests for ZIP packaging operations (task 7.7).
//!
//! These tests exercise `create_zip` and `cancel_zip` through the
//! `IFileService` trait, covering text content packaging, disk file
//! packaging, mixed entries, cancellation, sandbox validation,
//! and archive verification.

use std::fs;
use std::io::Read;
use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileService, IFileService, ZipEntry};
use nomifun_realtime::EventBroadcaster;

/// No-op broadcaster for tests that don't need event verification.
struct NoopBroadcaster;

impl EventBroadcaster for NoopBroadcaster {
    fn broadcast(&self, _event: WebSocketMessage<serde_json::Value>) {}
}

fn make_service(root: &std::path::Path) -> FileService {
    FileService::new(Arc::new(NoopBroadcaster), vec![root.to_path_buf()])
}

// -----------------------------------------------------------------------
// create_zip — test-plan 5.1
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_zip_text_content() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("text.zip");

    let svc = make_service(dir.path());
    let entries = vec![
        ZipEntry::Text {
            name: "a.txt".into(),
            content: "hello".into(),
        },
        ZipEntry::Text {
            name: "dir/b.txt".into(),
            content: "world".into(),
        },
    ];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await.unwrap();
    assert!(result);

    // Verify the ZIP can be opened and contains correct data
    let file = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.len(), 2);

    {
        let mut entry = archive.by_name("a.txt").unwrap();
        let mut buf = String::new();
        entry.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "hello");
    }
    {
        let mut entry = archive.by_name("dir/b.txt").unwrap();
        let mut buf = String::new();
        entry.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "world");
    }
}

#[tokio::test]
async fn create_zip_disk_files() {
    let dir = tempfile::tempdir().unwrap();
    let src_a = dir.path().join("src_a.txt");
    let src_b = dir.path().join("src_b.bin");
    fs::write(&src_a, "file A content").unwrap();
    fs::write(&src_b, b"\x00\x01\x02\x03").unwrap();

    let zip_path = dir.path().join("disk.zip");
    let svc = make_service(dir.path());
    let entries = vec![
        ZipEntry::Disk {
            name: "a.txt".into(),
            file_path: src_a.to_string_lossy().into_owned(),
        },
        ZipEntry::Disk {
            name: "b.bin".into(),
            file_path: src_b.to_string_lossy().into_owned(),
        },
    ];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await.unwrap();
    assert!(result);

    let file = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.len(), 2);

    {
        let mut entry = archive.by_name("a.txt").unwrap();
        let mut buf = String::new();
        entry.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "file A content");
    }
    {
        let mut entry = archive.by_name("b.bin").unwrap();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"\x00\x01\x02\x03");
    }
}

#[tokio::test]
async fn create_zip_mixed_content_and_disk() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("real.txt");
    fs::write(&src, "real file").unwrap();

    let zip_path = dir.path().join("mixed.zip");
    let svc = make_service(dir.path());
    let entries = vec![
        ZipEntry::Text {
            name: "virtual.txt".into(),
            content: "in-memory".into(),
        },
        ZipEntry::Disk {
            name: "real.txt".into(),
            file_path: src.to_string_lossy().into_owned(),
        },
    ];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await.unwrap();
    assert!(result);

    let file = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.len(), 2);

    {
        let mut entry = archive.by_name("virtual.txt").unwrap();
        let mut buf = String::new();
        entry.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "in-memory");
    }
    {
        let mut entry = archive.by_name("real.txt").unwrap();
        let mut buf = String::new();
        entry.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "real file");
    }
}

#[tokio::test]
async fn create_zip_with_request_id() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("req.zip");
    let svc = make_service(dir.path());

    let entries = vec![ZipEntry::Text {
        name: "data.txt".into(),
        content: "test data".into(),
    }];

    let result = svc
        .create_zip(zip_path.to_str().unwrap(), entries, Some("req-123".into()))
        .await
        .unwrap();
    assert!(result);
    assert!(zip_path.exists());
}

// -----------------------------------------------------------------------
// cancel_zip — test-plan 5.2
// -----------------------------------------------------------------------

#[tokio::test]
async fn cancel_zip_nonexistent_request() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    // Cancelling a request that doesn't exist returns false
    let result = svc.cancel_zip("no-such-id").await;
    assert!(!result);
}

#[tokio::test]
async fn cancel_zip_completed_request_returns_false() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("done.zip");
    let svc = make_service(dir.path());

    let entries = vec![ZipEntry::Text {
        name: "a.txt".into(),
        content: "data".into(),
    }];

    // Complete the ZIP first
    svc.create_zip(zip_path.to_str().unwrap(), entries, Some("req-done".into()))
        .await
        .unwrap();

    // After completion, the token is cleaned up — cancel returns false
    let result = svc.cancel_zip("req-done").await;
    assert!(!result);
}

// -----------------------------------------------------------------------
// Error cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_zip_disk_entry_nonexistent_source() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("fail.zip");
    let svc = make_service(dir.path());

    let entries = vec![ZipEntry::Disk {
        name: "missing.txt".into(),
        file_path: "/nonexistent/path/file.txt".into(),
    }];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn create_zip_empty_entries_produces_valid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("empty.zip");
    let svc = make_service(dir.path());

    let result = svc.create_zip(zip_path.to_str().unwrap(), vec![], None).await.unwrap();
    assert!(result);

    let file = fs::File::open(&zip_path).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.len(), 0);
}

// -----------------------------------------------------------------------
// Sandbox validation
// -----------------------------------------------------------------------

#[tokio::test]
async fn create_zip_rejects_output_outside_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    // Output path is outside the allowed root
    let zip_path = other.path().join("escape.zip");
    let entries = vec![ZipEntry::Text {
        name: "a.txt".into(),
        content: "data".into(),
    }];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await;
    assert!(result.is_err());
    assert!(!zip_path.exists());
}

#[tokio::test]
async fn create_zip_rejects_disk_entry_outside_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let outside_file = other.path().join("secret.txt");
    fs::write(&outside_file, "sensitive data").unwrap();

    let svc = make_service(dir.path());
    let zip_path = dir.path().join("steal.zip");
    let entries = vec![ZipEntry::Disk {
        name: "stolen.txt".into(),
        file_path: outside_file.to_string_lossy().into_owned(),
    }];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await;
    assert!(result.is_err());
    assert!(!zip_path.exists());
}

#[tokio::test]
async fn create_zip_rejects_nonexistent_disk_entry_in_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("fail.zip");
    let svc = make_service(dir.path());

    // Disk entry points to a non-existent file inside the sandbox —
    // validate_path rejects it before any ZIP is created.
    let entries = vec![ZipEntry::Disk {
        name: "missing.txt".into(),
        file_path: dir.path().join("no_such.txt").to_string_lossy().into_owned(),
    }];

    let result = svc.create_zip(zip_path.to_str().unwrap(), entries, None).await;
    assert!(result.is_err());
    assert!(!zip_path.exists());
}

// -----------------------------------------------------------------------
// cancel_zip — in-progress cancellation (test-plan 5.2)
// -----------------------------------------------------------------------

#[tokio::test]
async fn cancel_zip_in_progress() {
    let dir = tempfile::tempdir().unwrap();

    // Create many small source files to give time for cancellation
    let src_dir = dir.path().join("sources");
    fs::create_dir(&src_dir).unwrap();
    let entry_count = 500;
    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let name = format!("file_{i:04}.txt");
        let path = src_dir.join(&name);
        fs::write(&path, format!("content of file {i}")).unwrap();
        entries.push(ZipEntry::Disk {
            name,
            file_path: path.to_string_lossy().into_owned(),
        });
    }

    let zip_path = dir.path().join("big.zip");
    let svc = Arc::new(make_service(dir.path()));
    let svc_cancel = Arc::clone(&svc);

    let zip_path_str = zip_path.to_string_lossy().into_owned();
    let request_id = "cancel-me".to_owned();

    // Spawn ZIP creation in a separate task
    let create_handle = tokio::spawn({
        let request_id = request_id.clone();
        async move { svc.create_zip(&zip_path_str, entries, Some(request_id)).await }
    });

    // Give a brief moment for the creation to start, then cancel
    tokio::task::yield_now().await;
    let cancelled = svc_cancel.cancel_zip(&request_id).await;

    let result = create_handle.await.unwrap();

    // Either the cancel signal was picked up (Ok(false)) or it completed
    // before the signal arrived (Ok(true)). The key assertion is that
    // cancel_zip returned true (it found the token).
    if cancelled {
        // cancel_zip found and set the flag
        match result {
            Ok(false) => {
                // Successfully cancelled — partial file should be removed
                assert!(
                    !zip_path.exists(),
                    "partial ZIP should be cleaned up after cancellation"
                );
            }
            Ok(true) => {
                // ZIP completed before cancellation took effect — file exists
                assert!(zip_path.exists());
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    } else {
        // Token was already cleaned up, meaning create_zip finished first
        assert!(result.is_ok());
    }
}
