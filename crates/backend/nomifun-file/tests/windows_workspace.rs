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

fn creation(path: &str) -> AgentSessionFilePatch {
    AgentSessionFilePatch {
        path: path.into(), expected_source: AgentSessionPatchSource::Absent,
        hunks: vec![AgentSessionPatchHunk {
            old_start: 0, old_lines: 0, new_start: 1, new_lines: 1,
            lines: vec![AgentSessionPatchLine::Add { text: "new file".into() }],
        }],
    }
}

#[tokio::test]
async fn new_case_aliases_reject_before_any_publication_or_directory_creation() {
    for paths in [
        ["Report.txt", "REPORT.TXT"],
        ["fresh/Report.txt", "FRESH/report.txt"],
        ["new", "NEW/child.txt"],
        ["NEW/child.txt", "new"],
        ["Résumé.txt", "RÉSUMÉ.TXT"],
    ] {
        let root = tempfile::tempdir().unwrap();
        let (service, scope, events) = owner(root.path());
        let result = service.apply_patch_with_observation_for_agent_session(&scope,
            AgentSessionPatchRequest { files: paths.iter().map(|path| creation(path)).collect() }).await;
        let failure = result.unwrap_err();
        assert!(failure.observation.published.is_empty(),
            "aliases must fail before publication: {paths:?}, {:?}", failure.observation);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0,
            "aliases must not create any target or parent: {paths:?}");
        assert!(events.0.lock().unwrap().is_empty());
    }
}

fn set_case_sensitive(path: &Path, enabled: bool) {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES,
        FileCaseSensitiveInfo, SetFileInformationByHandle,
    };
    use windows_sys::Win32::System::SystemServices::FILE_CS_FLAG_CASE_SENSITIVE_DIR;
    let directory = fs::OpenOptions::new().write(true).access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path).unwrap();
    let info = FILE_CASE_SENSITIVE_INFO { Flags: if enabled { FILE_CS_FLAG_CASE_SENSITIVE_DIR } else { 0 } };
    let ok = unsafe { SetFileInformationByHandle(directory.as_raw_handle(), FileCaseSensitiveInfo,
        (&info as *const FILE_CASE_SENSITIVE_INFO).cast(), std::mem::size_of_val(&info) as u32) };
    assert_ne!(ok, 0, "case-sensitive NTFS fixture unavailable: {}", std::io::Error::last_os_error());
}

fn icacls(path: &Path, arguments: &[&std::ffi::OsStr]) {
    let output = std::process::Command::new("icacls.exe").arg(path).args(arguments)
        .output().expect("Windows ACL fixture requires icacls.exe");
    assert!(output.status.success(), "ACL fixture failed: {} {}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

fn saved_dacl(path: &Path) -> Vec<u8> {
    let evidence = tempfile::tempdir().unwrap();
    let saved = evidence.path().join("dacl.txt");
    icacls(path, &["/save".as_ref(), saved.as_os_str()]);
    fs::read(saved).unwrap()
}

#[tokio::test]
async fn metadata_and_nested_search_need_no_parent_listing_or_synchronize_access() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir_all(parent.join("child")).unwrap();
    fs::create_dir(parent.join(".GIT")).unwrap();
    let target = parent.join("Known.TXT");
    fs::write(&target, b"metadata").unwrap();
    fs::write(parent.join("child/.GITIGNORE"), b"skip.txt\n").unwrap();
    fs::write(parent.join("child/keep.txt"), b"needle").unwrap();
    fs::write(parent.join("child/skip.txt"), b"needle").unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(&parent, &["/deny".as_ref(), "*S-1-1-0:(RD,S)".as_ref()]);
    icacls(&target, &["/deny".as_ref(), "*S-1-1-0:(RD)".as_ref()]);
    let native = fs::metadata(&target);
    let forbidden_read = fs::read(&target);
    let forbidden_listing = fs::read_dir(&parent);
    let metadata = service.get_file_metadata_for_agent_session(&scope, "parent/KNOWN.txt").await;
    let instruction = service.instruction_scope_for_agent_session(&scope,
        nomifun_file::AgentInstructionScopeRequest { path: "parent/Known.TXT".into(), recursive: false }).await;
    let search = service.search_text_for_agent_session(&scope,
        nomifun_file::AgentTextSearchRequest { path: Some("parent/child".into()), query: "needle".into(), limit: None }).await;
    icacls(&target, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    icacls(&parent, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    assert_eq!(native.unwrap().len(), 8);
    assert!(forbidden_read.is_err());
    assert!(forbidden_listing.is_err());
    let metadata = metadata.unwrap();
    assert_eq!(metadata.size, 8);
    assert_eq!(metadata.name, "Known.TXT");
    assert!(metadata.last_modified > 0);
    assert_eq!(instruction.unwrap().kind, "file");
    let search = search.unwrap();
    assert!(!search.truncated, "{:?}", search.incomplete_reasons);
    assert_eq!(search.matches.iter().map(|item| item.path.as_str()).collect::<Vec<_>>(), ["parent/child/keep.txt"]);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn denied_metadata_does_not_become_missing_scope() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_FLAG_OPEN_REPARSE_POINT};
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("denied.txt");
    fs::write(&target, b"preserved").unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(&target, &["/deny".as_ref(), "*S-1-1-0:(RA)".as_ref()]);
    icacls(&parent, &["/deny".as_ref(), "*S-1-1-0:(RD)".as_ref()]);
    let native = fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&target);
    let metadata = service.get_file_metadata_for_agent_session(&scope, "parent/denied.txt").await;
    let instruction = service.instruction_scope_for_agent_session(&scope,
        nomifun_file::AgentInstructionScopeRequest { path: "parent/denied.txt".into(), recursive: false }).await;
    icacls(&parent, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    icacls(&target, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    assert_eq!(native.err().expect("fixture must deny an independent native attribute open").raw_os_error(), Some(5));
    assert!(!matches!(metadata.err().expect("denied metadata must fail"), nomifun_common::AppError::NotFound(_)));
    assert!(instruction.is_err(), "permission denial must not return a missing scope");
    assert_eq!(fs::read(&target).unwrap(), b"preserved");
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn file_delete_respects_readonly_and_keeps_old_readers_after_success() {
    use std::io::Read;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    let permissions = fs::metadata(&target).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&target, readonly).unwrap();
    let rejected = service.remove_entry_for_agent_session(&scope, "target.txt").await;
    fs::set_permissions(&target, permissions).unwrap();
    assert!(rejected.is_err());
    assert!(!nomifun_file::file_delete_outcome_unknown(&rejected.unwrap_err()));
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(events.0.lock().unwrap().is_empty());
    let mut reader = lock(&target, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    service.remove_entry_for_agent_session(&scope, "target.txt").await.unwrap();
    assert!(!target.exists());
    assert_eq!(events.0.lock().unwrap().len(), 1);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original");
    fs::write(&target, b"new owner").unwrap();
    drop(reader);
    assert_eq!(fs::read(&target).unwrap(), b"new owner");
}

#[tokio::test]
async fn file_delete_needs_neither_parent_listing_nor_file_data_read() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("target.txt");
    let control = parent.join("native.txt");
    fs::write(&target, b"target").unwrap();
    fs::write(&control, b"control").unwrap();
    let (service, scope, events) = owner(root.path());
    for path in [&parent, &target, &control] {
        icacls(path, &["/deny".as_ref(), "*S-1-1-0:(RD)".as_ref()]);
    }
    let read = fs::read(&target);
    let listing = fs::read_dir(&parent);
    let native = fs::remove_file(&control);
    let deleted = service.remove_entry_for_agent_session(&scope, "parent/target.txt").await;
    icacls(&parent, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    if target.exists() { icacls(&target, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]); }
    if control.exists() { icacls(&control, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]); }
    assert!(read.is_err());
    assert!(listing.is_err());
    native.unwrap();
    deleted.unwrap();
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn recursive_delete_pages_wide_and_deep_unicode_trees_without_following_junctions() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    let tree = root.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for index in 0..300 {
        fs::write(tree.join(format!("{index:04}-空 格🐱-{}.txt", "x".repeat(200))), b"remove").unwrap();
    }
    let deep = tree.join(vec!["目录-中文"; 40].join("/"));
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("last.txt"), b"deep").unwrap();
    junction::create(outside.path(), tree.join("outside-link")).unwrap();
    fs::write(root.path().join("keep.txt"), b"sibling").unwrap();
    let (service, scope, events) = owner(root.path());
    service.remove_entry_for_agent_session(&scope, "tree").await.unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read(outside.path().join("sentinel")).unwrap(), b"outside");
    assert_eq!(fs::read(root.path().join("keep.txt")).unwrap(), b"sibling");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn empty_directory_delete_needs_no_directory_listing_permission() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let control = root.path().join("native");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&control).unwrap();
    let (service, scope, events) = owner(root.path());
    for path in [&target, &control] {
        icacls(path, &["/deny".as_ref(), "*S-1-1-0:(RD)".as_ref()]);
    }
    let listing = fs::read_dir(&target);
    let native = fs::remove_dir(&control);
    let deleted = service.remove_entry_for_agent_session(&scope, "target").await;
    for path in [&target, &control] {
        if path.exists() { icacls(path, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]); }
    }
    assert!(listing.is_err());
    native.unwrap();
    deleted.unwrap();
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reading_a_known_file_does_not_require_directory_listing_or_lock_out_other_readers() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ,FILE_SHARE_WRITE,FILE_SHARE_DELETE};
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    let target = nested.join("known.txt");
    fs::write(&target,b"readable").unwrap();
    let (service,scope,_) = owner(root.path());
    let _existing = lock(&target,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE);
    icacls(&nested,&["/deny".as_ref(),"*S-1-1-0:(RD)".as_ref()]);
    let native = fs::read(&target);
    let listing = fs::read_dir(&nested);
    let read = service.read_bytes_for_agent_session(&scope,"nested/known.txt",64).await;
    icacls(&nested,&["/remove:d".as_ref(),"*S-1-1-0".as_ref()]);
    assert_eq!(native.unwrap(),b"readable","known file is readable without listing its parent");
    assert!(listing.is_err(),"fixture must deny parent listing");
    assert_eq!(read.unwrap().unwrap().0,b"readable");
}

#[tokio::test]
async fn writes_do_not_require_parent_listing_or_write_access_to_existing_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let ancestor = root.path().join("ancestor");
    let parent = ancestor.join("parent");
    fs::create_dir_all(&parent).unwrap();
    fs::write(parent.join("known.txt"), b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(&ancestor, &["/deny".as_ref(), "*S-1-1-0:(W)".as_ref()]);
    icacls(&parent, &["/deny".as_ref(), "*S-1-1-0:(RD,X)".as_ref()]);
    let listing = fs::read_dir(&parent);
    let forbidden_sibling = fs::write(ancestor.join("denied.txt"), b"denied");
    let native_write = fs::write(parent.join("native.txt"), b"allowed");
    let write = service.write_file_for_agent_session(&scope, "ancestor/parent/known.txt", b"replacement").await;
    let patch = service.apply_patch_for_agent_session(&scope,
        replacement("ancestor/parent/known.txt", "replacement", "patched")).await;
    let create = service.write_file_for_agent_session(&scope, "ancestor/parent/new/child.txt", b"created").await;
    icacls(&parent, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    icacls(&ancestor, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    assert!(listing.is_err());
    assert!(forbidden_sibling.is_err());
    native_write.unwrap();
    assert!(!write.unwrap());
    patch.unwrap();
    assert!(create.unwrap());
    assert_eq!(fs::read(parent.join("known.txt")).unwrap(), b"patched");
    assert_eq!(fs::read(parent.join("new/child.txt")).unwrap(), b"created");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 3);
    assert_eq!(fs::read_dir(parent.join("new")).unwrap().count(), 1);
    assert_eq!(events.0.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn atomic_publication_respects_directory_synchronize_denial() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("known.txt"), b"original").unwrap();
    let original = parent.join("native-original.txt");
    let staged = parent.join("native-staged.txt");
    let backup = parent.join("native-backup.txt");
    let linked = parent.join("native-linked.txt");
    fs::write(&original, b"native original").unwrap();
    fs::write(&staged, b"native staged").unwrap();
    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let original_wide = wide(&original);
    let staged_wide = wide(&staged);
    let backup_wide = wide(&backup);
    let native_replace = || {
        // SAFETY: all three NUL-terminated paths remain live for the call.
        let ok = unsafe { ReplaceFileW(original_wide.as_ptr(), staged_wide.as_ptr(), backup_wide.as_ptr(),
            0, std::ptr::null(), std::ptr::null()) };
        if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    };
    let (service, scope, events) = owner(root.path());
    icacls(&parent, &["/deny".as_ref(), "*S-1-1-0:(S)".as_ref()]);
    let native_in_place = fs::write(&original, b"native original");
    let replacement = native_replace();
    let creation = fs::hard_link(&staged, &linked);
    let write = service.write_file_for_agent_session(&scope, "parent/known.txt", b"replacement").await;
    let create = service.write_file_for_agent_session(&scope, "parent/new.txt", b"created").await;
    icacls(&parent, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    native_in_place.unwrap();
    assert_eq!(replacement.unwrap_err().raw_os_error(), Some(1175));
    assert_eq!(creation.unwrap_err().raw_os_error(), Some(5));
    assert!(write.is_err());
    assert!(create.is_err());
    assert_eq!(fs::read(parent.join("known.txt")).unwrap(), b"original");
    assert_eq!(fs::read(&original).unwrap(), b"native original");
    assert_eq!(fs::read(&staged).unwrap(), b"native staged");
    assert!(!parent.join("new.txt").exists());
    assert!(!linked.exists());
    assert!(!backup.exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 3);
    assert!(events.0.lock().unwrap().is_empty());
    fs::hard_link(&staged, &linked).unwrap();
    native_replace().unwrap(); // Confirm both reference operations work after fixture release.
}

#[tokio::test]
async fn workspace_root_junction_and_internal_alias_keep_their_authorized_read_target() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("workspace");
    let directory = root.join("actual");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("value.txt"),b"owned").unwrap();
    junction::create(&directory,root.join("alias")).unwrap();
    let root_alias = fixture.path().join("workspace-alias");
    junction::create(&root,&root_alias).unwrap();
    let (service,scope,_) = owner(&root_alias);
    let read = service.read_bytes_for_agent_session(&scope,"alias/value.txt",64).await.unwrap().unwrap();
    assert_eq!(read.0,b"owned");
}

#[tokio::test]
async fn acl_deny_write_preserves_existing_file_for_write_and_patch() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("protected.txt");
    fs::write(&target, b"original").unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(&target, &["/deny".as_ref(), "*S-1-1-0:(W)".as_ref()]);
    let dacl = saved_dacl(&target);
    let written = service.write_file_for_agent_session(&scope, "protected.txt", b"wrong").await;
    let patched = service.apply_patch_with_observation_for_agent_session(&scope,
        replacement("protected.txt", "original", "wrong")).await;
    let unchanged_dacl = dacl == saved_dacl(&target);
    icacls(&target, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    assert!(written.is_err());
    let failure = patched.unwrap_err();
    assert!(failure.observation.published.is_empty());
    assert!(failure.observation.temporary_cleanup_unconfirmed.is_empty());
    assert!(unchanged_dacl);
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn acl_deny_parent_creation_leaves_no_files_or_directories() {
    let root = tempfile::tempdir().unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(root.path(), &["/deny".as_ref(), "*S-1-1-0:(W)".as_ref()]);
    let written = service.write_file_for_agent_session(&scope, "new.txt", b"wrong").await;
    let patched = service.apply_patch_with_observation_for_agent_session(&scope,
        AgentSessionPatchRequest { files: vec![creation("nested/new.txt")] }).await;
    icacls(root.path(), &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    assert!(written.is_err());
    let failure = patched.unwrap_err();
    assert!(failure.observation.published.is_empty());
    assert!(failure.observation.temporary_cleanup_unconfirmed.is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn acl_denied_entry_and_parent_delete_preserves_target_contents() {
    for is_directory in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("protected");
        let content = if is_directory {
            fs::create_dir(&target).unwrap();
            target.join("keep.txt")
        } else { target.clone() };
        fs::write(&content, b"original").unwrap();
        let (service, scope, events) = owner(root.path());
        // Windows grants deletion through either file DELETE or parent DELETE_CHILD;
        // the negative fixture must deny both access paths.
        icacls(root.path(), &["/deny".as_ref(), "*S-1-1-0:(DC)".as_ref()]);
        icacls(&target, &["/deny".as_ref(), "*S-1-1-0:(DE)".as_ref()]);
        let removed = service.remove_entry_for_agent_session(&scope, "protected").await;
        icacls(root.path(), &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
        icacls(&target, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
        assert!(removed.is_err());
        assert!(!nomifun_file::file_delete_outcome_unknown(removed.as_ref().unwrap_err()));
        assert_eq!(fs::read(&content).unwrap(), b"original", "directory={is_directory}");
        assert!(events.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn acl_and_named_stream_survive_successful_write_and_patch() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("custom.txt");
    let stream = root.path().join("custom.txt:metadata");
    fs::write(&target, b"original").unwrap();
    fs::write(&stream, b"native stream fixture").unwrap();
    // A non-default, narrower DACL: writing is allowed, execution is denied.
    icacls(&target, &["/deny".as_ref(), "*S-1-1-0:(X)".as_ref()]);
    let dacl = saved_dacl(&target);
    let (service, scope, events) = owner(root.path());
    assert!(!service.write_file_for_agent_session(&scope, "custom.txt", b"written").await.unwrap());
    assert_eq!(fs::read(&target).unwrap(), b"written");
    assert_eq!(saved_dacl(&target), dacl);
    assert_eq!(fs::read(&stream).unwrap(), b"native stream fixture");
    service.apply_patch_for_agent_session(&scope, replacement("custom.txt", "written", "patched")).await.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"patched");
    assert_eq!(saved_dacl(&target), dacl);
    assert_eq!(fs::read(&stream).unwrap(), b"native stream fixture");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert_eq!(events.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn acl_failure_inside_recursive_delete_requires_remaining_tree_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("a-removable.txt"), b"removable").unwrap();
    let denied = tree.join("z-denied.txt");
    fs::write(&denied, b"keep").unwrap();
    let (service, scope, events) = owner(root.path());
    icacls(&tree, &["/deny".as_ref(), "*S-1-1-0:(DC)".as_ref()]);
    icacls(&denied, &["/deny".as_ref(), "*S-1-1-0:(DE)".as_ref()]);
    assert_eq!(service.list_workspace_files_for_agent_session(&scope).await.unwrap().len(), 2);
    let removed = service.remove_entry_for_agent_session(&scope, "tree").await;
    icacls(&tree, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    icacls(&denied, &["/remove:d".as_ref(), "*S-1-1-0".as_ref()]);
    let error = removed.unwrap_err();
    assert!(nomifun_file::file_delete_outcome_unknown(&error), "{error}");
    assert_eq!(fs::read(&denied).unwrap(), b"keep");
    assert!(events.0.lock().unwrap().is_empty(), "partial deletion is not a successful delete event");
    // Other entries may already be gone; the error is never a zero-effect proof.
    let listed = service.list_workspace_files_for_agent_session(&scope).await.unwrap();
    assert!(listed.iter().any(|file| file.relative_path == "tree/z-denied.txt"));
    assert_eq!(listed.len(), fs::read_dir(&tree).unwrap().count(), "reconciliation must not use stale cached entries");
}

#[tokio::test]
async fn case_sensitive_directories_preserve_distinct_names_and_inherited_children() {
    let root = tempfile::tempdir().unwrap();
    set_case_sensitive(root.path(), true);
    let (service, scope, _) = owner(root.path());
    for paths in [["Report.txt", "report.txt"], ["new/Report.txt", "new/report.txt"]] {
        let receipt = service.apply_patch_for_agent_session(&scope,
            AgentSessionPatchRequest { files: paths.iter().map(|path| creation(path)).collect() }).await.unwrap();
        assert_eq!(receipt.files.len(), 2);
        let first = fs::canonicalize(root.path().join(paths[0])).unwrap();
        let second = fs::canonicalize(root.path().join(paths[1])).unwrap();
        assert_ne!(first, second);
        assert!(!same_file::is_same_file(&first, &second).unwrap());
        for path in paths { assert_eq!(fs::read(root.path().join(path)).unwrap(), b"new file"); }
    }
}

#[tokio::test]
async fn each_parent_case_flag_controls_new_target_identity() {
    let root = tempfile::tempdir().unwrap();
    set_case_sensitive(root.path(), true);
    let insensitive = root.path().join("insensitive");
    fs::create_dir(&insensitive).unwrap();
    set_case_sensitive(&insensitive, false);
    let (service, scope, events) = owner(root.path());
    let failure = service.apply_patch_with_observation_for_agent_session(&scope,
        AgentSessionPatchRequest { files: ["insensitive/New/a.txt", "insensitive/new/A.txt"]
            .iter().map(|path| creation(path)).collect() }).await.unwrap_err();
    assert!(failure.observation.published.is_empty());
    assert_eq!(fs::read_dir(&insensitive).unwrap().count(), 0);
    assert!(events.0.lock().unwrap().is_empty());
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

#[tokio::test]
async fn owner_path_observations_resolve_junctions_case_and_unicode_without_exposing_the_root() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("real")).unwrap();
    junction::create(root.path().join("real"), root.path().join("shortcut")).unwrap();
    let (service, scope, _) = owner(root.path());
    let written = service.write_file_with_observation_for_agent_session(&scope, "real/验收-Report.txt", b"original").await.unwrap();
    let identity = written.workspace_path.unwrap();
    assert_eq!(identity.path, "real/验收-Report.txt");
    assert!(identity.case_resolved);
    let page = service.read_text_page_for_agent_session(&scope, serde_json::from_value(serde_json::json!({
        "path":"shortcut/验收-REPORT.TXT"
    })).unwrap()).await.unwrap().unwrap();
    assert_eq!(page.path, "shortcut/验收-REPORT.TXT");
    assert_eq!(page.workspace_path.as_ref(), Some(&identity));
    assert!(!serde_json::to_string(&page).unwrap().contains(root.path().to_str().unwrap()));
    let patch = service.apply_patch_for_agent_session(&scope, replacement("shortcut/验收-report.txt", "original", "patched")).await.unwrap();
    assert_eq!(patch.files[0].workspace_path.as_ref(), Some(&identity));
    let deleted = service.remove_entry_with_observation_for_agent_session(&scope, "shortcut/验收-REPORT.TXT").await.unwrap();
    assert_eq!(deleted, Some(identity));
    assert!(!root.path().join("real/验收-Report.txt").exists());
}

#[tokio::test]
async fn canonical_path_metadata_is_inside_the_text_page_wire_budget() {
    let root = tempfile::tempdir().unwrap();
    let relative = format!("{}/source.txt", vec!["long-parent-中文"; 20].join("/"));
    let target = root.path().join(&relative);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "\"\\\n".repeat(12_000)).unwrap();
    junction::create(target.parent().unwrap(), root.path().join("shortcut")).unwrap();
    let (service, scope, _) = owner(root.path());
    let page = service.read_text_page_for_agent_session(&scope, serde_json::from_value(serde_json::json!({
        "path":"shortcut/source.txt"
    })).unwrap()).await.unwrap().unwrap();
    assert_eq!(page.workspace_path.as_ref().unwrap().path, relative);
    let encoded = serde_json::to_string(&page).unwrap();
    assert!(encoded.len() <= 24 * 1024);
    assert!(serde_json::to_vec(&encoded).unwrap().len() <= 24 * 1024);
    assert!(!page.eof && page.next_offset.is_some_and(|offset| offset > 0));
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
