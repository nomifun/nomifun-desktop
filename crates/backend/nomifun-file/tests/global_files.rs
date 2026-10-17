//! The canonical Session file owner uses native paths; workspace identity is
//! retained only for project observations. These are deterministic native
//! filesystem regressions, not Agent/model or packaged-App acceptance.
use std::{fs, path::Path, sync::{Arc, Mutex}};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{
    AgentSessionFilePatch, AgentSessionPatchHunk, AgentSessionPatchLine,
    AgentSessionPatchRequest, AgentSessionPatchSource, AgentTextReadRequest,
    AgentTextSearchRequest, FileService, WORKSPACE_DELETE_OPERATION,
    WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, workspace_binding,
};
use nomifun_realtime::UserEventSink;
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Events(Mutex<Vec<WebSocketMessage<serde_json::Value>>>);
impl UserEventSink for Events {
    fn send_to_user(&self, _: &str, event: WebSocketMessage<serde_json::Value>) {
        self.0.lock().unwrap().push(event);
    }
}

fn replacement(path: &str, old: &str, new: &str) -> AgentSessionPatchRequest {
    AgentSessionPatchRequest { files: vec![AgentSessionFilePatch {
        path: path.into(),
        expected_source: AgentSessionPatchSource::Existing { sha256: format!("{:x}", Sha256::digest(old)) },
        hunks: vec![AgentSessionPatchHunk {
            old_start: 1, old_lines: 1, new_start: 1, new_lines: 1,
            lines: vec![AgentSessionPatchLine::Remove { text: old.into() }, AgentSessionPatchLine::Add { text: new.into() }],
        }],
    }] }
}

#[tokio::test]
async fn canonical_owner_reads_searches_patches_renames_and_deletes_outside_default_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let events = Arc::new(Events::default());
    let service = FileService::new(events.clone(), vec![workspace.clone()]);
    let scope = workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, WORKSPACE_DELETE_OPERATION], &workspace).unwrap();
    let relative = "../外部目录 with spaces/nested/test.txt";
    let target = fixture.path().join("外部目录 with spaces/nested/test.txt");
    let written = service.write_file_with_observation_for_agent_session(&scope, relative, b"before").await.unwrap();
    assert!(written.created);
    assert!(written.workspace_path.is_none());
    assert_eq!(service.read_file_for_agent_session(&scope, target.to_str().unwrap()).await.unwrap().as_deref(), Some("before"));
    let page = service.read_text_page_for_agent_session(&scope, AgentTextReadRequest {
        path: relative.into(), offset: 0, limit: 16_384, expected_sha256: None, start_line: None, line_count: None,
    }).await.unwrap().unwrap();
    assert_eq!(page.content, "before");
    assert!(page.workspace_path.is_none());
    let result = service.apply_patch_for_agent_session(&scope, replacement(relative, "before", "after")).await.unwrap();
    assert_eq!(result.files[0].path, fs::canonicalize(&target).unwrap().to_str().unwrap());
    assert!(result.files[0].workspace_path.is_none());
    let stale = service.apply_patch_for_agent_session(&scope, replacement(relative, "before", "clobber")).await.unwrap_err();
    assert!(stale.to_string().contains("source"));
    let search = service.search_text_for_agent_session(&scope, AgentTextSearchRequest {
        query: "after".into(), path: Some("../外部目录 with spaces".into()), limit: None,
    }).await.unwrap();
    assert!(!search.truncated, "{:?}", search.incomplete_reasons);
    assert_eq!(search.matches.len(), 1);
    assert!(Path::new(&search.matches[0].path).is_absolute());
    assert_eq!(fs::canonicalize(&target).unwrap(), Path::new(&search.matches[0].path));
    let renamed = service.rename_entry_for_agent_session(&scope, relative, "../外部目录 with spaces/nested/改名.txt").await.unwrap();
    assert_eq!(fs::read_to_string(&renamed).unwrap(), "after");
    service.remove_entry_for_agent_session(&scope, &renamed).await.unwrap();
    assert!(!Path::new(&renamed).exists());
    service.remove_entry_for_agent_session(&scope, "../外部目录 with spaces").await.unwrap();
    assert!(!fixture.path().join("外部目录 with spaces").exists());
    assert!(events.0.lock().unwrap().is_empty(), "external effects must not become project events");
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_owner_follows_content_links_but_renames_and_unlinks_the_link_entry() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("project");
    let external = fixture.path().join("external");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&external).unwrap();
    let target = external.join("file.txt");
    fs::write(&target, "before").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, workspace.join("link")).unwrap();
    symlink(&external, workspace.join("directory-link")).unwrap();
    let service = FileService::new(Arc::new(Events::default()), vec![]);
    let scope = workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, WORKSPACE_DELETE_OPERATION], &workspace).unwrap();
    service.write_file_for_agent_session(&scope, "link", b"written").await.unwrap();
    service.apply_patch_for_agent_session(&scope, replacement("link", "written", "patched")).await.unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "patched");
    assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
    assert!(fs::symlink_metadata(workspace.join("link")).unwrap().file_type().is_symlink());
    service.rename_entry_for_agent_session(&scope, "link", "renamed-link").await.unwrap();
    assert!(fs::symlink_metadata(workspace.join("renamed-link")).unwrap().file_type().is_symlink());
    service.remove_entry_for_agent_session(&scope, "renamed-link").await.unwrap();
    service.remove_entry_for_agent_session(&scope, "directory-link").await.unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "patched");
    assert!(external.is_dir());
    fs::write(external.join("occupied"), b"preserve").unwrap();
    assert!(service.rename_entry_for_agent_session(&scope, "../external/file.txt", "../external/occupied").await.is_err());
    assert_eq!(fs::read(external.join("occupied")).unwrap(), b"preserve");
    assert_eq!(fs::read(&target).unwrap(), b"patched");
    symlink("missing", external.join("occupied-dangling")).unwrap();
    assert!(service.rename_entry_for_agent_session(&scope, "../external/file.txt", "../external/occupied-dangling").await.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"patched");
    // Native Unix names retain their backslash and colon bytes.
    service.write_file_for_agent_session(&scope, r"../external/native\name:part", b"native").await.unwrap();
    assert_eq!(fs::read(external.join(r"native\name:part")).unwrap(), b"native");
    symlink("missing-target", workspace.join("dangling")).unwrap();
    service.remove_entry_for_agent_session(&scope, "dangling").await.unwrap();
    assert!(!fs::symlink_metadata(workspace.join("dangling")).is_ok());
}

#[tokio::test]
async fn absolute_file_operations_remain_available_after_default_directory_is_removed() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("deleted-default");
    fs::create_dir(&workspace).unwrap();
    let scope = workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION, WORKSPACE_DELETE_OPERATION], &workspace).unwrap();
    fs::remove_dir(&workspace).unwrap();
    let external = fixture.path().join("external.txt").to_string_lossy().into_owned();
    let service = FileService::new(Arc::new(Events::default()), vec![]);
    service.write_file_for_agent_session(&scope, &external, b"before").await.unwrap();
    service.apply_patch_for_agent_session(&scope, replacement(&external, "before", "after")).await.unwrap();
    let scope_result = service.instruction_scope_for_agent_session(&scope,
        nomifun_file::AgentInstructionScopeRequest { path: external.clone(), recursive: false }).await.unwrap();
    assert_eq!(scope_result.canonical_path, fs::canonicalize(&external).unwrap().to_str().unwrap());
    let search = service.search_text_for_agent_session(&scope, AgentTextSearchRequest { query: "after".into(), path: Some(external.clone()), limit: None }).await.unwrap();
    assert_eq!(search.matches.len(), 1);
    service.remove_entry_for_agent_session(&scope, &external).await.unwrap();
    assert!(!workspace.exists());
}

#[tokio::test]
async fn different_session_defaults_cannot_lose_each_others_patch_to_one_external_file() {
    let fixture = tempfile::tempdir().unwrap();
    let first_root = fixture.path().join("first");
    let second_root = fixture.path().join("second");
    fs::create_dir(&first_root).unwrap(); fs::create_dir(&second_root).unwrap();
    let target = fixture.path().join("shared.txt").to_string_lossy().into_owned();
    fs::write(&target, "before").unwrap();
    let binding = |root: &Path| workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION], root).unwrap();
    let first = FileService::new(Arc::new(Events::default()), vec![]);
    let second = FileService::new(Arc::new(Events::default()), vec![]);
    let first_scope = binding(&first_root); let second_scope = binding(&second_root);
    let (left, right) = tokio::join!(
        first.apply_patch_for_agent_session(&first_scope, replacement(&target, "before", "first")),
        second.apply_patch_for_agent_session(&second_scope, replacement(&target, "before", "second")),
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert!(matches!(fs::read_to_string(&target).unwrap().as_str(), "first" | "second"));
}

#[cfg(unix)]
#[tokio::test]
async fn rename_destination_parent_access_denial_preserves_the_native_error_and_source() {
    use std::os::unix::fs::PermissionsExt;
    use nomifun_common::AppError;

    struct RestorePermissions {
        path: std::path::PathBuf,
        permissions: fs::Permissions,
    }
    impl Drop for RestorePermissions {
        fn drop(&mut self) { fs::set_permissions(&self.path, self.permissions.clone()).unwrap(); }
    }

    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("readable-source.txt");
    fs::write(&source, b"preserve original source").unwrap();
    let denied = fixture.path().join("private-parent");
    let child = denied.join("child");
    fs::create_dir_all(&child).unwrap();
    let restore = RestorePermissions { path: denied.clone(), permissions: fs::metadata(&denied).unwrap().permissions() };
    let scope = workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
        [WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION], fixture.path()).unwrap();
    let service = FileService::new(Arc::new(Events::default()), vec![]);
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
    // The child requires traversing the denied directory. Resolving the
    // directory itself can succeed without that permission on some Unix hosts.
    let native_denial = fs::canonicalize(&child)
        .expect_err("permission fixture requires a login user that cannot traverse mode 000");
    assert_eq!(native_denial.kind(), std::io::ErrorKind::PermissionDenied);
    let destination = child.join("moved.txt");
    let error = service.rename_entry_for_agent_session(&scope,
        source.to_str().unwrap(), destination.to_str().unwrap()).await.unwrap_err();
    drop(restore);

    assert_eq!(error.error_code(), "SYSTEM_ACCESS_DENIED");
    match &error {
        AppError::NativeSystemAccessDenied { source, .. } => {
            assert_eq!(source.kind(), native_denial.kind());
            assert_eq!(source.raw_os_error(), native_denial.raw_os_error());
        }
        other => panic!("native traversal denial was misclassified: {other:?}"),
    }
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(fs::read(&source).unwrap(), b"preserve original source");
    assert!(!destination.exists());
    let invalid = fixture.path().join("invalid\0name");
    assert!(matches!(nomifun_file::path_safety::validate_path_for_write_authority(
        invalid.to_str().unwrap(), &nomifun_file::PathAuthority::Unrestricted), Err(AppError::BadRequest(_))),
        "invalid file-name parameters must remain a payload rejection");
}
