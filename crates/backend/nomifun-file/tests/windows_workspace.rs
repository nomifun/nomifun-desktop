//! Native Windows workspace-owner regressions, not full Agent/UI acceptance.
#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{
    AgentSessionFilePatch, AgentSessionPatchHunk, AgentSessionPatchLine,
    AgentSessionPatchRequest, AgentSessionPatchSource,
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

fn replacement(path: &str, before: &str, after: &str) -> AgentSessionPatchRequest {
    use sha2::{Digest, Sha256};
    AgentSessionPatchRequest { files: vec![AgentSessionFilePatch {
        path: path.into(),
        expected_source: AgentSessionPatchSource::Existing {
            sha256: format!("{:x}", Sha256::digest(before.as_bytes())),
        },
        hunks: vec![AgentSessionPatchHunk {
            old_start: 1, old_lines: 1, new_start: 1, new_lines: 1,
            lines: vec![
                AgentSessionPatchLine::Remove { text: before.into() },
                AgentSessionPatchLine::Add { text: after.into() },
            ],
        }],
    }] }
}

fn lock(path: &Path, sharing: u32) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).share_mode(sharing).open(path).unwrap()
}

#[tokio::test]
async fn write_rejects_deny_delete_lock_and_preserves_original_until_explicit_retry() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("locked.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    let locker = lock(&target, FILE_SHARE_READ | FILE_SHARE_WRITE);
    let result = service.write_file_for_agent_session(&scope, "locked.txt", b"replacement").await;
    assert!(result.is_err(), "deny-delete write unexpectedly succeeded: {result:?}");
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "temporary file leaked");
    assert!(events.0.lock().unwrap().is_empty());
    drop(locker);
    let created = service.write_file_for_agent_session(&scope, "locked.txt", b"replacement").await.unwrap();
    assert!(!created, "replacing an existing file must not report creation");
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn patch_and_delete_reject_deny_delete_lock_without_events_or_residue() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("locked.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    let locker = lock(&target, FILE_SHARE_READ | FILE_SHARE_WRITE);
    let failure = service.apply_patch_with_observation_for_agent_session(&scope,
        replacement("locked.txt", "original", "replacement")).await.unwrap_err();
    assert_eq!(failure.observation.failed_file, Some(0));
    assert!(failure.observation.published.is_empty());
    assert!(failure.observation.temporary_cleanup_unconfirmed.is_empty());
    assert!(service.remove_entry_for_agent_session(&scope, "locked.txt").await.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
    drop(locker);
    service.apply_patch_for_agent_session(&scope, replacement("locked.txt", "original", "replacement")).await.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    service.remove_entry_for_agent_session(&scope, "locked.txt").await.unwrap();
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(events.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn write_and_patch_respect_deny_write_even_when_delete_sharing_is_allowed() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("locked.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    let _locker = lock(&target, FILE_SHARE_READ | FILE_SHARE_DELETE);
    assert!(service.write_file_for_agent_session(&scope, "locked.txt", b"replacement").await.is_err());
    let result = service.apply_patch_with_observation_for_agent_session(&scope,
        replacement("locked.txt", "original", "replacement")).await;
    assert!(result.is_err(), "deny-write patch unexpectedly succeeded: {result:?}");
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn successful_write_replaces_file_instead_of_truncating_the_open_original() {
    use std::io::Read;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("output.txt");
    let (service, scope, events) = owner(root.path());
    assert!(service.write_file_for_agent_session(&scope, "output.txt", b"original").await.unwrap());
    let mut original = lock(&target, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    let created = service.write_file_for_agent_session(&scope, "output.txt", b"new").await.unwrap();
    let mut old_bytes = Vec::new();
    original.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, b"original", "write truncated the original file identity");
    assert!(!created);
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert_eq!(events.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn readonly_target_rejects_write_and_patch_without_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("readonly.txt");
    fs::write(&target, b"original").unwrap();
    let original_permissions = fs::metadata(&target).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&target, readonly).unwrap();
    let (service, scope, events) = owner(root.path());
    let written = service.write_file_for_agent_session(&scope, "readonly.txt", b"wrong").await;
    let patched = service.apply_patch_with_observation_for_agent_session(&scope,
        replacement("readonly.txt", "original", "wrong")).await;
    // Restore the fixture's original attributes before an assertion can panic.
    fs::set_permissions(&target, original_permissions).unwrap();
    assert!(written.is_err());
    let failure = patched.unwrap_err();
    assert!(failure.observation.published.is_empty());
    assert!(failure.observation.temporary_cleanup_unconfirmed.is_empty());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
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

#[tokio::test]
async fn deleting_a_junction_never_recurses_into_its_in_root_target() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("keep.txt"), b"keep").unwrap();
    junction::create(&real, root.path().join("alias")).unwrap();
    let (service, scope, events) = owner(root.path());
    let result = service.remove_entry_for_agent_session(&scope, "alias").await;
    assert!(result.is_err(), "junction deletion must explicitly reject: {result:?}");
    assert_eq!(fs::read(real.join("keep.txt")).unwrap(), b"keep");
    assert!(junction::exists(root.path().join("alias")).unwrap());
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn deleting_workspace_root_is_rejected_even_with_delete_authority() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("keep.txt"), b"keep").unwrap();
    let (service, scope, events) = owner(root.path());
    assert!(service.remove_entry_for_agent_session(&scope, "").await.is_err());
    assert_eq!(fs::read(root.path().join("keep.txt")).unwrap(), b"keep");
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn outside_junction_rejects_file_actions_and_parent_deletion_only_removes_the_link() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
    fs::create_dir(root.path().join("parent")).unwrap();
    junction::create(outside.path(), root.path().join("parent/escape")).unwrap();
    let (service, scope, events) = owner(root.path());
    assert!(service.read_bytes_for_agent_session(&scope, "parent/escape/keep.txt", 1024).await.is_err());
    assert!(service.write_file_for_agent_session(&scope, "parent/escape/keep.txt", b"wrong").await.is_err());
    assert!(service.apply_patch_for_agent_session(&scope,
        replacement("parent/escape/keep.txt", "keep", "wrong")).await.is_err());
    assert!(service.remove_entry_for_agent_session(&scope, "parent/escape").await.is_err());
    assert!(events.0.lock().unwrap().is_empty());
    service.remove_entry_for_agent_session(&scope, "parent").await.unwrap();
    assert!(!root.path().join("parent").exists());
    assert_eq!(fs::read(outside.path().join("keep.txt")).unwrap(), b"keep");
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn existing_case_aliases_in_one_patch_reject_before_any_publication() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("Report.txt"), b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    let mut request = replacement("Report.txt", "original", "first");
    request.files.extend(replacement("REPORT.TXT", "original", "second").files);
    let failure = service.apply_patch_with_observation_for_agent_session(&scope, request).await.unwrap_err();
    assert!(failure.observation.published.is_empty());
    assert_eq!(fs::read(root.path().join("Report.txt")).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn long_native_paths_and_maximum_ntfs_component_round_trip_without_shortening() {
    let root = tempfile::tempdir().unwrap();
    let name = format!("{}.txt", "x".repeat(251));
    let relative = format!("{}/{}", vec!["nested-space-中文"; 24].join("/"), name);
    assert!(root.path().join(&relative).as_os_str().len() > 260);
    let (service, scope, events) = owner(root.path());
    assert!(service.write_file_for_agent_session(&scope, &relative, b"original").await.unwrap());
    assert!(!service.write_file_for_agent_session(&scope, &relative, b"replacement").await.unwrap());
    let receipt = service.apply_patch_for_agent_session(&scope,
        replacement(&relative, "replacement", "patched")).await.unwrap();
    assert_eq!(receipt.files[0].path, relative);
    assert_eq!(service.read_bytes_for_agent_session(&scope, &relative, 1024).await.unwrap().unwrap().0, b"patched");
    let target = fs::canonicalize(root.path().join(&relative)).unwrap();
    assert_eq!(target.file_name().unwrap().to_str().unwrap(), name);
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
    service.remove_entry_for_agent_session(&scope, &relative).await.unwrap();
    assert!(!target.exists());
    assert_eq!(events.0.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn overlong_component_is_a_stable_error_without_truncated_sibling_effects() {
    let root = tempfile::tempdir().unwrap();
    let maximum = "x".repeat(255);
    let overlong = "x".repeat(256);
    fs::write(root.path().join(&maximum), b"keep").unwrap();
    let (service, scope, events) = owner(root.path());
    let first = service.write_file_for_agent_session(&scope, &overlong, b"wrong").await.unwrap_err();
    let second = service.write_file_for_agent_session(&scope, &overlong, b"wrong").await.unwrap_err();
    assert_eq!(first.to_string(), second.to_string());
    assert_eq!(fs::read(root.path().join(&maximum)).unwrap(), b"keep");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}
