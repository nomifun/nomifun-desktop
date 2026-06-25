use std::sync::Arc;

use nomifun_api_types::ToolType;
use nomifun_shell::{NoopSystemOpener, ShellService};

fn service() -> ShellService {
    ShellService::new(Arc::new(NoopSystemOpener))
}

// ---------------------------------------------------------------------------
// SH-2: open_file — file does not exist
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh2_open_file_not_found() {
    let err = service().open_file("/nonexistent/file.txt").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("not found") || msg.contains("does not exist"),
        "expected 'not found', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// SH-4: show_item_in_folder — path does not exist
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh4_show_item_in_folder_not_found() {
    let err = service().show_item_in_folder("/nonexistent/path").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not found"), "expected 'not found', got: {msg}");
}

// ---------------------------------------------------------------------------
// SH-6: open_external — command injection attempt
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh6_open_external_command_injection() {
    let err = service().open_external("; rm -rf /").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("invalid") || msg.to_lowercase().contains("url"),
        "expected 'invalid' or 'URL', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// SH-7: open_external — disallowed scheme (file://)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh7_open_external_file_scheme() {
    let err = service().open_external("file:///etc/passwd").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("scheme") || msg.to_lowercase().contains("not allowed"),
        "expected scheme error, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// SH-8: check_tool_installed — terminal always true
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh8_check_tool_terminal_always_true() {
    assert!(service().check_tool_installed(ToolType::Terminal).await);
}

// ---------------------------------------------------------------------------
// SH-9: check_tool_installed — explorer always true
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh9_check_tool_explorer_always_true() {
    assert!(service().check_tool_installed(ToolType::Explorer).await);
}

// ---------------------------------------------------------------------------
// SH-10: check_tool_installed — vscode (environment-dependent)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh10_check_tool_vscode_returns_bool() {
    let _installed = service().check_tool_installed(ToolType::Vscode).await;
}

// ---------------------------------------------------------------------------
// SH-12: open_folder_with — directory does not exist
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh12_open_folder_with_dir_not_found() {
    let err = service()
        .open_folder_with("/nonexistent/dir", ToolType::Explorer)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not found"), "expected 'not found', got: {msg}");
}

// ---------------------------------------------------------------------------
// SH-13: open_file — missing filePath (tested via empty string)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh13_open_file_empty_path() {
    let err = service().open_file("").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("not found"),
        "expected 'not found' for empty path, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// SH-14: open_external — empty string
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sh14_open_external_empty_url() {
    let err = service().open_external("").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("invalid") || msg.to_lowercase().contains("url"),
        "expected invalid URL error, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Additional: open_folder_with — file path instead of directory
// ---------------------------------------------------------------------------
#[tokio::test]
async fn open_folder_with_file_path_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "data").unwrap();
    let err = service()
        .open_folder_with(file.to_str().unwrap(), ToolType::Explorer)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("directory not found"),
        "expected 'directory not found', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Additional: open_external — ftp scheme rejected
// ---------------------------------------------------------------------------
#[tokio::test]
async fn open_external_ftp_scheme_rejected() {
    let err = service().open_external("ftp://evil.com/file").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("scheme"), "expected scheme error, got: {msg}");
}

// ---------------------------------------------------------------------------
// Additional: open_external — javascript scheme rejected
// ---------------------------------------------------------------------------
#[tokio::test]
async fn open_external_javascript_scheme_rejected() {
    let err = service().open_external("javascript:alert(1)").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("scheme") || msg.to_lowercase().contains("invalid"),
        "expected scheme or invalid error, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Error conversion: ShellError → AppError mapping
// ---------------------------------------------------------------------------
#[test]
fn shell_error_converts_to_app_error() {
    use nomifun_common::AppError;
    use nomifun_shell::ShellError;

    let err: AppError = ShellError::FileNotFound("/tmp/x".into()).into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = ShellError::DirectoryNotFound("/tmp/y".into()).into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = ShellError::InvalidUrl("bad".into()).into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = ShellError::ToolNotInstalled("vscode".into()).into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = ShellError::CommandFailed("oops".into()).into();
    assert!(matches!(err, AppError::Internal(_)));
}
