//! Native macOS workspace-owner regressions, not full Agent/UI acceptance.
#![cfg(target_os = "macos")]

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::Command,
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

fn add_acl(path: &Path, entry: &str) {
    let output = Command::new("/bin/chmod")
        .arg("+a")
        .arg(entry)
        .arg(path)
        .output()
        .expect("macOS ACL fixture requires /bin/chmod");
    assert!(
        output.status.success(),
        "failed to add ACL to {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn acl_listing(path: &Path) -> String {
    let output = Command::new("/bin/ls")
        .arg("-lde")
        .arg(path)
        .output()
        .expect("macOS ACL fixture requires /bin/ls");
    assert!(output.status.success(), "failed to list ACL for {}", path.display());
    String::from_utf8(output.stdout).expect("ACL listing is UTF-8")
}

fn write_xattr(path: &Path, name: &str, value: &str) {
    let output = Command::new("/usr/bin/xattr")
        .arg("-w")
        .arg(name)
        .arg(value)
        .arg(path)
        .output()
        .expect("macOS xattr fixture requires /usr/bin/xattr");
    assert!(
        output.status.success(),
        "failed to write xattr on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_xattr(path: &Path, name: &str) -> String {
    let output = Command::new("/usr/bin/xattr")
        .arg("-p")
        .arg(name)
        .arg(path)
        .output()
        .expect("macOS xattr fixture requires /usr/bin/xattr");
    assert!(
        output.status.success(),
        "failed to read xattr on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = String::from_utf8(output.stdout).expect("xattr value is UTF-8");
    value.strip_suffix('\n').unwrap_or(&value).to_owned()
}

fn set_file_flag(path: &Path, flag: &str) {
    let output = Command::new("/usr/bin/chflags")
        .arg(flag)
        .arg(path)
        .output()
        .expect("macOS flag fixture requires /usr/bin/chflags");
    assert!(
        output.status.success(),
        "failed to set {flag} on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn file_flags(path: &Path) -> String {
    let output = Command::new("/usr/bin/stat")
        .arg("-f")
        .arg("%Sf")
        .arg(path)
        .output()
        .expect("macOS flag fixture requires /usr/bin/stat");
    assert!(output.status.success(), "failed to read flags for {}", path.display());
    String::from_utf8(output.stdout)
        .expect("flag listing is UTF-8")
        .trim()
        .to_owned()
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
    assert!(format!("{:?}", write.unwrap_err()).contains("publication outcome is unknown"));
    assert!(format!("{:?}", patch.unwrap_err()).contains("publication outcome is unknown"));
    assert_eq!(fs::read(&write_target).unwrap(), b"original");
    assert_eq!(fs::read(&patch_target).unwrap(), b"original");
    for target in [&write_target, &patch_target] {
        assert_eq!(fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o444);
    }
    let retained = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nomifun-patch-"))
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 2, "each denied publication retains one owned stage");
    for entry in retained {
        assert_eq!(fs::read(entry.path()).unwrap(), b"wrong");
    }
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 2, "each uncertain denial requires one reconciliation event");
    for event in events.iter() {
        assert_eq!(event.name, "fileStream.contentUpdate");
        assert_eq!(event.data["operation"], "write");
        assert!(event.data.get("content").is_none(), "uncertain failure cannot publish intended bytes");
    }
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
async fn successful_write_and_patch_preserve_extended_acl() {
    let root = tempfile::tempdir().unwrap();
    let write_target = root.path().join("write-acl.txt");
    let patch_target = root.path().join("patch-acl.txt");
    fs::write(&write_target, b"original").unwrap();
    fs::write(&patch_target, b"original").unwrap();
    for target in [&write_target, &patch_target] {
        add_acl(target, "everyone deny execute");
        assert!(acl_listing(target).contains("group:everyone deny execute"));
    }
    let (service, scope, _) = owner(root.path());

    service
        .write_file_for_agent_session(&scope, "write-acl.txt", b"replacement")
        .await
        .unwrap();
    service
        .apply_patch_for_agent_session(
            &scope,
            replacement("patch-acl.txt", "original", "replacement"),
        )
        .await
        .unwrap();

    assert_eq!(fs::read(&write_target).unwrap(), b"replacement");
    assert_eq!(fs::read(&patch_target).unwrap(), b"replacement");
    for target in [&write_target, &patch_target] {
        assert!(
            acl_listing(target).contains("group:everyone deny execute"),
            "successful publication dropped the target ACL: {}",
            target.display()
        );
    }
}

#[tokio::test]
async fn successful_write_and_patch_preserve_extended_attributes() {
    const XATTR_NAME: &str = "com.nomifun.reliability.fixture";
    const XATTR_VALUE: &str = "M03-XATTR-SENTINEL";
    const RESOURCE_FORK: &str = "com.apple.ResourceFork";
    const RESOURCE_VALUE: &str = "M03-RESOURCE-FORK-SENTINEL";

    let root = tempfile::tempdir().unwrap();
    let write_target = root.path().join("write-xattr.txt");
    let patch_target = root.path().join("patch-xattr.txt");
    fs::write(&write_target, b"original").unwrap();
    fs::write(&patch_target, b"original").unwrap();
    for target in [&write_target, &patch_target] {
        write_xattr(target, XATTR_NAME, XATTR_VALUE);
        write_xattr(target, RESOURCE_FORK, RESOURCE_VALUE);
        assert_eq!(read_xattr(target, XATTR_NAME), XATTR_VALUE);
        assert_eq!(read_xattr(target, RESOURCE_FORK), RESOURCE_VALUE);
    }
    let (service, scope, _) = owner(root.path());

    service
        .write_file_for_agent_session(&scope, "write-xattr.txt", b"replacement")
        .await
        .unwrap();
    service
        .apply_patch_for_agent_session(
            &scope,
            replacement("patch-xattr.txt", "original", "replacement"),
        )
        .await
        .unwrap();

    assert_eq!(fs::read(&write_target).unwrap(), b"replacement");
    assert_eq!(fs::read(&patch_target).unwrap(), b"replacement");
    for target in [&write_target, &patch_target] {
        assert_eq!(read_xattr(target, XATTR_NAME), XATTR_VALUE);
        assert_eq!(read_xattr(target, RESOURCE_FORK), RESOURCE_VALUE);
    }
}

#[tokio::test]
async fn acl_deny_write_rejects_write_and_patch_without_changing_bytes() {
    let root = tempfile::tempdir().unwrap();
    let write_target = root.path().join("write-denied.txt");
    let patch_target = root.path().join("patch-denied.txt");
    fs::write(&write_target, b"original").unwrap();
    fs::write(&patch_target, b"original").unwrap();
    for target in [&write_target, &patch_target] {
        add_acl(target, "everyone deny write");
        assert!(acl_listing(target).contains("group:everyone deny write"));
    }
    let (service, scope, events) = owner(root.path());

    let write = service
        .write_file_for_agent_session(&scope, "write-denied.txt", b"wrong")
        .await;
    let patch = service
        .apply_patch_for_agent_session(
            &scope,
            replacement("patch-denied.txt", "original", "wrong"),
        )
        .await;

    assert!(write.is_err(), "ACL-denied write unexpectedly succeeded: {write:?}");
    assert!(patch.is_err(), "ACL-denied patch unexpectedly succeeded: {patch:?}");
    assert!(format!("{:?}", write.unwrap_err()).contains("publication outcome is unknown"));
    assert!(format!("{:?}", patch.unwrap_err()).contains("publication outcome is unknown"));
    assert_eq!(fs::read(&write_target).unwrap(), b"original");
    assert_eq!(fs::read(&patch_target).unwrap(), b"original");
    for target in [&write_target, &patch_target] {
        assert!(acl_listing(target).contains("group:everyone deny write"));
    }
    let retained = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nomifun-patch-"))
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 2, "each denied publication retains one owned stage");
    for entry in retained {
        assert_eq!(fs::read(entry.path()).unwrap(), b"wrong");
    }
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 2, "each uncertain denial requires one reconciliation event");
    for event in events.iter() {
        assert_eq!(event.name, "fileStream.contentUpdate");
        assert_eq!(event.data["operation"], "write");
        assert!(event.data.get("content").is_none(), "uncertain failure cannot publish intended bytes");
    }
}

#[tokio::test]
async fn immutable_targets_reject_write_and_patch_without_losing_flags() {
    let root = tempfile::tempdir().unwrap();
    let write_target = root.path().join("write-immutable.txt");
    let patch_target = root.path().join("patch-immutable.txt");
    fs::write(&write_target, b"original").unwrap();
    fs::write(&patch_target, b"original").unwrap();
    for target in [&write_target, &patch_target] {
        set_file_flag(target, "uchg");
        assert!(file_flags(target).contains("uchg"));
    }
    let (service, scope, events) = owner(root.path());

    let write = service
        .write_file_for_agent_session(&scope, "write-immutable.txt", b"wrong")
        .await;
    let patch = service
        .apply_patch_for_agent_session(
            &scope,
            replacement("patch-immutable.txt", "original", "wrong"),
        )
        .await;

    let write_bytes = fs::read(&write_target).unwrap();
    let patch_bytes = fs::read(&patch_target).unwrap();
    let write_flags = file_flags(&write_target);
    let patch_flags = file_flags(&patch_target);
    let retained = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nomifun-patch-"))
        .collect::<Vec<_>>();
    let events = events.0.lock().unwrap().clone();
    for target in [&write_target, &patch_target] {
        set_file_flag(target, "nouchg");
    }

    assert!(write.is_err(), "immutable write unexpectedly succeeded: {write:?}");
    assert!(patch.is_err(), "immutable patch unexpectedly succeeded: {patch:?}");
    assert!(format!("{:?}", write.unwrap_err()).contains("publication outcome is unknown"));
    assert!(format!("{:?}", patch.unwrap_err()).contains("publication outcome is unknown"));
    assert_eq!(write_bytes, b"original");
    assert_eq!(patch_bytes, b"original");
    assert!(write_flags.contains("uchg"));
    assert!(patch_flags.contains("uchg"));
    assert_eq!(retained.len(), 2, "each immutable denial retains one owned stage");
    for entry in retained {
        assert_eq!(fs::read(entry.path()).unwrap(), b"wrong");
    }
    assert_eq!(events.len(), 2, "each uncertain denial requires one reconciliation event");
    for event in events {
        assert_eq!(event.name, "fileStream.contentUpdate");
        assert_eq!(event.data["operation"], "write");
        assert!(event.data.get("content").is_none(), "uncertain failure cannot publish intended bytes");
    }
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
