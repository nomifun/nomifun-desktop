//! Integration tests for image processing operations (task 7.6).
//!
//! These tests exercise `get_image_base64` and `fetch_remote_image`
//! through the `IFileService` trait, covering local image encoding,
//! remote image fetching with whitelist/protocol/size validation,
//! and placeholder SVG fallback behavior.

use std::fs;
use std::sync::Arc;

use base64::Engine;

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{FileService, IFileService};
use nomifun_realtime::UserEventSink;

/// No-op broadcaster for tests that don't need event verification.
struct NoopBroadcaster;

impl UserEventSink for NoopBroadcaster {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
}

fn make_service(root: &std::path::Path) -> FileService {
    FileService::new(Arc::new(NoopBroadcaster), vec![root.to_path_buf()])
}

// -----------------------------------------------------------------------
// getImageBase64 — test-plan 4.1
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_image_base64_png() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.png");
    // Minimal valid-looking PNG bytes (magic header)
    let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    fs::write(&file, &png_bytes).unwrap();

    let svc = make_service(dir.path());
    let result = svc.get_image_base64(file.to_str().unwrap(), None).await.unwrap();

    assert!(
        result.starts_with("data:image/png;base64,"),
        "expected data:image/png;base64, prefix, got: {}",
        &result[..50.min(result.len())]
    );

    // Verify roundtrip: decode base64 back to original bytes
    let encoded_part = result.strip_prefix("data:image/png;base64,").unwrap();
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded_part).unwrap();
    assert_eq!(decoded, png_bytes);
}

#[tokio::test]
async fn get_image_base64_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("photo.jpg");
    let jpeg_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    fs::write(&file, &jpeg_bytes).unwrap();

    let svc = make_service(dir.path());
    let result = svc.get_image_base64(file.to_str().unwrap(), None).await.unwrap();

    assert!(result.starts_with("data:image/jpeg;base64,"));
}

#[tokio::test]
async fn get_image_base64_svg() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("icon.svg");
    let svg_content =
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><circle cx="50" cy="50" r="40"/></svg>"#;
    fs::write(&file, svg_content).unwrap();

    let svc = make_service(dir.path());
    let result = svc.get_image_base64(file.to_str().unwrap(), None).await.unwrap();

    assert!(result.starts_with("data:image/svg+xml;base64,"));

    // Verify content roundtrip
    let encoded_part = result.strip_prefix("data:image/svg+xml;base64,").unwrap();
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded_part).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), svg_content);
}

#[tokio::test]
async fn get_image_base64_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());
    let result = svc
        .get_image_base64(dir.path().join("missing.png").to_str().unwrap(), None)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn image_base64_with_extra_workspace_root() {
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("test.png");
    let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    fs::write(&file, &png_bytes).unwrap();

    let svc = make_service(sandbox.path());
    let result = svc
        .get_image_base64(file.to_str().unwrap(), Some(workspace.path()))
        .await;

    assert!(result.unwrap().starts_with("data:image/png;base64,"));
}

#[tokio::test]
async fn get_image_base64_gif() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("animation.gif");
    let gif_bytes = b"GIF89a\x01\x00\x01\x00\x80\x00\x00";
    fs::write(&file, gif_bytes).unwrap();

    let svc = make_service(dir.path());
    let result = svc.get_image_base64(file.to_str().unwrap(), None).await.unwrap();

    assert!(result.starts_with("data:image/gif;base64,"));
}

#[tokio::test]
async fn get_image_base64_path_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());
    let result = svc.get_image_base64("../../etc/passwd", None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_image_base64_outside_sandbox_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());
    // /tmp exists but is outside the sandbox (dir.path())
    let result = svc.get_image_base64("/etc/hosts", None).await;

    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// fetchRemoteImage — test-plan 4.2
// -----------------------------------------------------------------------

#[tokio::test]
async fn fetch_remote_image_disallowed_host_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("https://evil.com/image.png").await;

    assert!(
        result.starts_with("data:image/svg+xml;base64,"),
        "expected placeholder SVG, got: {}",
        &result[..60.min(result.len())]
    );
}

#[tokio::test]
async fn fetch_remote_image_ftp_protocol_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("ftp://github.com/image.png").await;

    assert!(result.starts_with("data:image/svg+xml;base64,"));
}

#[tokio::test]
async fn fetch_remote_image_invalid_url_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("not-a-url").await;

    assert!(result.starts_with("data:image/svg+xml;base64,"));
}

#[tokio::test]
async fn fetch_remote_image_file_protocol_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("file:///etc/passwd").await;

    assert!(result.starts_with("data:image/svg+xml;base64,"));
}

#[tokio::test]
async fn fetch_remote_image_empty_url_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("").await;

    assert!(result.starts_with("data:image/svg+xml;base64,"));
}

#[tokio::test]
async fn fetch_remote_image_placeholder_contains_valid_svg() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("not-valid").await;

    // Verify the placeholder decodes to a valid SVG
    let encoded_part = result.strip_prefix("data:image/svg+xml;base64,").unwrap();
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded_part).unwrap();
    let svg = String::from_utf8(decoded).unwrap();
    assert!(svg.contains("<svg"));
    assert!(svg.contains("</svg>"));
}

#[tokio::test]
async fn fetch_remote_image_data_protocol_returns_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let svc = make_service(dir.path());

    let result = svc.fetch_remote_image("data:text/html,<script>alert(1)</script>").await;

    assert!(result.starts_with("data:image/svg+xml;base64,"));
}
