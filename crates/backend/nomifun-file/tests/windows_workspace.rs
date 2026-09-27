//! Native Windows workspace-owner regressions, not full Agent/UI acceptance.
#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{
    AgentSessionWorkspaceBinding, FileService, WORKSPACE_DELETE_OPERATION,
    WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, workspace_binding,
};
use nomifun_realtime::UserEventSink;

#[derive(Default)]
struct Events(Mutex<Vec<WebSocketMessage<serde_json::Value>>>);

impl UserEventSink for Events {
    fn send_to_user(&self, _owner: &str, event: WebSocketMessage<serde_json::Value>) {
        self.0.lock().unwrap().push(event);
    }
}

fn owner(root: &Path) -> (FileService, AgentSessionWorkspaceBinding, Arc<Events>) {
    let events = Arc::new(Events::default());
    let service = FileService::new(events.clone(), vec![root.to_path_buf()]);
    let scope = workspace_binding(
        nomifun_common::generate_id(),
        "windows-binding", "windows-workspace", "test-owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, WORKSPACE_DELETE_OPERATION],
        root,
    ).unwrap();
    (service, scope, events)
}

#[test]
fn rejects_all_win32_reserved_components() {
    use nomifun_file::path_safety::is_unsafe_path_segment;
    let accepted: Vec<_> = [
        "CON", "nul.txt", "PrN.tar.gz", "AUX", "COM1", "LPT9.log",
        "COM¹", "com².txt", "LPT³", "NUL .txt", "tail.", "tail ",
        "file:stream", "file::$DATA", "a?b", "a*b", "a<b", "a>b",
        "a|b", "a\"b", "a\u{1}b", "a\u{1f}b",
    ].into_iter().filter(|name| !is_unsafe_path_segment(name)).collect();
    assert!(accepted.is_empty(), "unsafe Windows components accepted: {accepted:?}");
    for name in ["COM0", "COM10", "LPT0", "console.txt", "null", ".env", "résumé 中文 🐱.txt"] {
        assert!(!is_unsafe_path_segment(name), "legal component rejected: {name}");
    }
}

#[tokio::test]
async fn rejects_existing_win32_aliases_without_reading_or_mutating_the_target() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    let target = root.path().join("nested/report.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    for alias in ["nested./report.txt", "nested /report.txt", "nested/report.txt."] {
        let result = service.read_bytes_for_agent_session(&scope, alias, 1024).await;
        assert!(result.is_err(), "alias read must reject {alias:?}: {result:?}");
        assert!(service.write_file_for_agent_session(&scope, alias, b"wrong").await.is_err());
        assert!(service.remove_entry_for_agent_session(&scope, alias).await.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(events.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn rejects_existing_ads_without_exposing_stream_bytes() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("report.txt");
    let stream = root.path().join("report.txt:private");
    fs::write(&target, b"main file").unwrap();
    fs::write(&stream, b"stream bytes").unwrap();
    let (service, scope, events) = owner(root.path());
    for path in ["report.txt:private", "report.txt::$DATA"] {
        let result = service.read_bytes_for_agent_session(&scope, path, 1024).await;
        assert!(result.is_err(), "ADS read must reject {path:?}: {result:?}");
        assert!(service.write_file_for_agent_session(&scope, path, b"wrong").await.is_err());
        assert!(service.remove_entry_for_agent_session(&scope, path).await.is_err());
    }
    assert_eq!(fs::read(&target).unwrap(), b"main file");
    assert_eq!(fs::read(&stream).unwrap(), b"stream bytes");
    assert!(events.0.lock().unwrap().is_empty());
}

#[test]
fn rejects_unc_device_absolute_and_drive_relative_input_before_root_resolution() {
    let root = tempfile::tempdir().unwrap();
    // An absent root makes filesystem resolution observable without touching
    // a network share or a device. Rejection must come from input validation.
    let (_, scope, _) = owner(&root.path().join("absent-root"));
    for path in [
        r"\\server\share\file", "//server/share/file", r"\\?\C:\file",
        r"\\.\NUL", "//?/C:/file", "//./NUL", "C:/file", "C:file",
        "/file", "../file", r"nested\file",
        "CON", "NUL.txt", "sub/COM¹.log", "sub/LPT³", "sub./file", "sub /file",
        "file:stream", "sub/file::$DATA", "sub/a?b", "sub/a\u{1}b",
    ] {
        let error = scope.resolve_relative_path(path).unwrap_err().to_string();
        assert!(error.contains("workspace path"), "{path:?}: {error}");
        assert!(!error.contains("cannot resolve"), "input reached filesystem: {path:?}: {error}");
    }
}

#[tokio::test]
async fn unicode_paths_and_case_aliases_resolve_to_one_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let (service, scope, _) = owner(root.path());
    let path = "空 格 🐱/Report.txt";
    service.write_file_for_agent_session(&scope, path, "内容\r\n".as_bytes()).await.unwrap();
    let canonical = service.get_file_metadata_for_agent_session(&scope, path).await.unwrap();
    let alias = service.get_file_metadata_for_agent_session(&scope, "空 格 🐱/REPORT.TXT").await.unwrap();
    assert_eq!(canonical.path, alias.path);
    assert_eq!(service.read_bytes_for_agent_session(&scope, "空 格 🐱/report.txt", 1024).await.unwrap().unwrap().0,
        "内容\r\n".as_bytes());
    assert_eq!(fs::read_dir(root.path().join("空 格 🐱")).unwrap().count(), 1);
}
