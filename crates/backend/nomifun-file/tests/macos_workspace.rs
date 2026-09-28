//! Native macOS workspace-owner regressions, not full Agent/UI acceptance.
#![cfg(target_os = "macos")]

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex},
};

use nomifun_api_types::WebSocketMessage;
use nomifun_file::{
    AgentSessionFilePatch, AgentSessionPatchHunk, AgentSessionPatchLine,
    AgentSessionPatchRequest, AgentSessionPatchSource, AgentSessionWorkspaceBinding, FileService,
    WORKSPACE_DELETE_OPERATION, WORKSPACE_READ_OPERATION, WORKSPACE_WRITE_OPERATION,
    workspace_binding,
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
        "macos-binding",
        "macos-workspace",
        "test-owner",
        [
            WORKSPACE_READ_OPERATION,
            WORKSPACE_WRITE_OPERATION,
            WORKSPACE_DELETE_OPERATION,
        ],
        root,
    )
    .unwrap();
    (service, scope, events)
}

fn creation(path: &str) -> AgentSessionFilePatch {
    AgentSessionFilePatch {
        path: path.into(),
        expected_source: AgentSessionPatchSource::Absent,
        hunks: vec![AgentSessionPatchHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: vec![AgentSessionPatchLine::Add {
                text: format!("created for {path}"),
            }],
        }],
    }
}

fn replacement(path: &str, before: &str, after: &str) -> AgentSessionPatchRequest {
    use sha2::{Digest, Sha256};

    AgentSessionPatchRequest {
        files: vec![AgentSessionFilePatch {
            path: path.into(),
            expected_source: AgentSessionPatchSource::Existing {
                sha256: format!("{:x}", Sha256::digest(before.as_bytes())),
            },
            hunks: vec![AgentSessionPatchHunk {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec![
                    AgentSessionPatchLine::Remove {
                        text: before.into(),
                    },
                    AgentSessionPatchLine::Add { text: after.into() },
                ],
            }],
        }],
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    let Ok(left) = fs::metadata(left) else {
        return false;
    };
    let Ok(right) = fs::metadata(right) else {
        return false;
    };
    left.dev() == right.dev() && left.ino() == right.ino()
}

async fn assert_alias_batch_rejected_before_publication(first: &str, second: &str) {
    let root = tempfile::tempdir().unwrap();
    let probe = root.path().join(first);
    fs::write(&probe, b"probe").unwrap();
    if !same_file(&probe, &root.path().join(second)) {
        // The case-sensitive APFS lane has a separate opt-in regression below.
        return;
    }
    fs::remove_file(&probe).unwrap();
    let (service, scope, events) = owner(root.path());

    let failure = service
        .apply_patch_with_observation_for_agent_session(
            &scope,
            AgentSessionPatchRequest {
                files: vec![creation(first), creation(second)],
            },
        )
        .await
        .expect_err("equivalent APFS targets must be rejected during preparation");

    assert!(
        failure.observation.published.is_empty(),
        "APFS aliases must fail before publication: {:?}",
        failure.observation
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn apfs_case_aliases_reject_before_any_publication() {
    assert_alias_batch_rejected_before_publication("Report.txt", "REPORT.TXT").await;
}

#[tokio::test]
async fn apfs_nfc_nfd_aliases_reject_before_any_publication() {
    assert_alias_batch_rejected_before_publication("é.txt", "e\u{301}.txt").await;
}

#[tokio::test]
async fn readonly_target_rejects_write_and_patch_without_changing_bytes() {
    let root = tempfile::tempdir().unwrap();
    let write_target = root.path().join("write-readonly.txt");
    let patch_target = root.path().join("patch-readonly.txt");
    fs::write(&write_target, b"original").unwrap();
    fs::write(&patch_target, b"original").unwrap();
    fs::set_permissions(&write_target, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&patch_target, fs::Permissions::from_mode(0o444)).unwrap();
    let (service, scope, events) = owner(root.path());

    let write = service
        .write_file_for_agent_session(&scope, "write-readonly.txt", b"wrong")
        .await;
    let patch = service
        .apply_patch_for_agent_session(
            &scope,
            replacement("patch-readonly.txt", "original", "wrong"),
        )
        .await;

    assert!(write.is_err(), "read-only write unexpectedly succeeded: {write:?}");
    assert!(patch.is_err(), "read-only patch unexpectedly succeeded: {patch:?}");
    assert_eq!(fs::read(&write_target).unwrap(), b"original");
    assert_eq!(fs::read(&patch_target).unwrap(), b"original");
    assert!(
        fs::read_dir(root.path())
            .unwrap()
            .all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".nomifun-patch-"))
    );
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn readonly_parent_rejects_create_and_preserves_siblings_without_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("readonly-parent");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("sibling.txt"), b"sibling").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let (service, scope, events) = owner(root.path());

    let result = service
        .write_file_for_agent_session(&scope, "readonly-parent/new.txt", b"wrong")
        .await;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(result.is_err(), "read-only parent unexpectedly accepted a create");
    assert_eq!(fs::read(parent.join("sibling.txt")).unwrap(), b"sibling");
    assert!(!parent.join("new.txt").exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires NOMIFUN_MACOS_CASE_SENSITIVE_TEST_ROOT on a case-sensitive APFS volume"]
async fn case_sensitive_apfs_preserves_distinct_case_and_unicode_names() {
    let parent = std::env::var_os("NOMIFUN_MACOS_CASE_SENSITIVE_TEST_ROOT")
        .expect("case-sensitive APFS root");
    let root = tempfile::tempdir_in(parent).unwrap();
    let (service, scope, _) = owner(root.path());
    let paths = ["Report.txt", "REPORT.TXT"];

    let result = service
        .apply_patch_for_agent_session(
            &scope,
            AgentSessionPatchRequest {
                files: paths.iter().map(|path| creation(path)).collect(),
            },
        )
        .await
        .unwrap();

    assert_eq!(result.file_count, paths.len());
    for path in paths {
        assert_eq!(fs::read_to_string(root.path().join(path)).unwrap(), format!("created for {path}"));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), paths.len());

    let unicode_root = tempfile::tempdir_in(
        std::env::var_os("NOMIFUN_MACOS_CASE_SENSITIVE_TEST_ROOT").unwrap(),
    )
    .unwrap();
    let (service, scope, events) = owner(unicode_root.path());
    let failure = service
        .apply_patch_with_observation_for_agent_session(
            &scope,
            AgentSessionPatchRequest {
                files: vec![creation("é.txt"), creation("e\u{301}.txt")],
            },
        )
        .await
        .expect_err("case-sensitive APFS still aliases canonical Unicode forms");
    assert!(failure.observation.published.is_empty());
    assert_eq!(fs::read_dir(unicode_root.path()).unwrap().count(), 0);
    assert!(events.0.lock().unwrap().is_empty());
}
