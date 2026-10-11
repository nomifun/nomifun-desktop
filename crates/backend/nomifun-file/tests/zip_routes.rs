//! Export routes use the host picker's destination policy without turning a
//! destination selection into authority to read arbitrary disk sources.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileRouterState, FileService, FileWatchService, SnapshotService, file_routes};
use nomifun_realtime::UserEventSink;
use serde_json::{Value, json};
use tower::ServiceExt;

struct NoopBroadcaster;

impl UserEventSink for NoopBroadcaster {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<Value>) {}
}

fn make_router(application_root: &Path, picker_root: &Path) -> axum::Router {
    let broadcaster = Arc::new(NoopBroadcaster);
    let roots = vec![application_root.to_path_buf()];
    let file_service = Arc::new(FileService::new(broadcaster.clone(), roots.clone()));
    let watch_service = Arc::new(FileWatchService::new(broadcaster, Arc::downgrade(&file_service)).unwrap());
    file_routes(FileRouterState {
        file_service,
        watch_service,
        snapshot_service: Arc::new(SnapshotService::new()),
        allowed_roots: roots,
        browse_roots: vec![picker_root.to_path_buf()],
    })
}

async fn create_zip(router: axum::Router, payload: Value) -> (StatusCode, Value) {
    post(router, "/api/fs/zip", payload).await
}

async fn post(router: axum::Router, uri: &str, payload: Value) -> (StatusCode, Value) {
    let response = router.oneshot(Request::builder()
        .method("POST").uri(uri).header("content-type", "application/json")
        .body(Body::from(payload.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn cancel_arriving_before_zip_body_prevents_later_publication() {
    let root = tempfile::tempdir().unwrap();
    let router = make_router(root.path(), root.path());
    let (status, response) = post(router.clone(), "/api/fs/zip/cancel", json!({
        "request_id": "body-still-arriving"
    })).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["data"], false, "there was no active worker yet");
    let output = root.path().join("early-canceled.zip");
    let (status, response) = create_zip(router, json!({
        "path": output, "request_id": "body-still-arriving",
        "files": [{ "name": "conversation.md", "content": "hello" }]
    })).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["data"], false);
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn selected_destination_outside_application_roots_packages_text_and_workspace_bytes() {
    let fixture = tempfile::tempdir().unwrap();
    let application = fixture.path().join("NomiFun");
    let workspace = fixture.path().join("project");
    std::fs::create_dir(&application).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    let source = workspace.join("asset.bin");
    let expected = [0, 255, 1, 0, 42];
    std::fs::write(&source, expected).unwrap();
    let output = fixture.path().join("conversation.zip");
    let (status, response) = create_zip(make_router(&application, fixture.path()), json!({
        "path": output,
        "workspaces": [workspace],
        "files": [
            { "name": "conversation/conversation.md", "content": "# hello" },
            { "name": "workspace/nested/asset.bin", "file_path": source }
        ]
    })).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["data"], true);
    let mut archive = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
    assert_eq!(archive.len(), 2);
    let mut actual = Vec::new();
    archive.by_name("workspace/nested/asset.bin").unwrap().read_to_end(&mut actual).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 3, "temporary ZIP must be removed");
}

#[tokio::test]
async fn destination_selection_does_not_admit_disk_sources() {
    let application = tempfile::tempdir().unwrap();
    let selected = tempfile::tempdir().unwrap();
    let source = selected.path().join("private.txt");
    std::fs::write(&source, "private").unwrap();
    let output = selected.path().join("conversation.zip");
    let (status, response) = create_zip(make_router(application.path(), selected.path()), json!({
        "path": output,
        "files": [{ "name": "private.txt", "file_path": source }]
    })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert!(!output.exists());
}

#[tokio::test]
async fn workspace_source_scope_does_not_admit_destination() {
    let application = tempfile::tempdir().unwrap();
    let selected = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let output = workspace.path().join("outside-picker.zip");
    let (status, response) = create_zip(make_router(application.path(), selected.path()), json!({
        "path": output,
        "workspaces": [workspace.path()],
        "files": [{ "name": "conversation.md", "content": "hello" }]
    })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert!(!output.exists());
}

#[cfg(any(windows, unix))]
#[tokio::test]
async fn workspace_links_cannot_escape_the_source_scope() {
    let application = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.txt"), "private").unwrap();
    let alias = workspace.path().join("alias");
    #[cfg(windows)]
    junction::create(outside.path(), &alias).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
    let output = application.path().join("escape.zip");
    let (status, response) = create_zip(make_router(application.path(), application.path()), json!({
        "path": output,
        "workspaces": [workspace.path()],
        "files": [{ "name": "private.txt", "file_path": alias.join("private.txt") }]
    })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert!(!output.exists());
}

#[tokio::test]
async fn malformed_disk_entries_fail_instead_of_exporting_empty_files() {
    let root = tempfile::tempdir().unwrap();
    for entry in [
        json!({ "name": "missing.txt" }),
        json!({ "name": "legacy.txt", "sourcePath": "anything" }),
        json!({ "name": "wrong.txt", "source_path": "anything" }),
        json!({ "name": "ambiguous.txt", "content": "hello", "file_path": "anything" }),
    ] {
        let output = root.path().join("invalid.zip");
        let (status, response) = create_zip(make_router(root.path(), root.path()), json!({
            "path": output, "files": [entry]
        })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert!(!output.exists());
    }
}
