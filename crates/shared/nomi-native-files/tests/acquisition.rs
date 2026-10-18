use std::{collections::BTreeSet, fs, path::Path};
use nomi_native_files::{NativeFileError, NativeReadAcquisition, NativeReadRequest, acquire, acquire_instruction_scope, acquire_source_path, acquire_source_with_adapter, acquire_source_with_hooks};
use sha2::{Digest, Sha256};

#[test]
fn source_acquisition_preserves_exact_bytes_hash_and_bounded_base64_wire() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.bin");
    let bytes = b"\xef\xbb\xbfline\r\n\0\xfftail";
    fs::write(&source, bytes).unwrap();
    let acquired = acquire_source_path(&source, 1024).unwrap().unwrap();
    assert_eq!(acquired.bytes, bytes);
    assert_eq!(acquired.sha256, format!("{:x}", Sha256::digest(bytes)));
    assert_eq!(Path::new(&acquired.canonical_path), fs::canonicalize(&source).unwrap());
    let wire = serde_json::to_value(&acquired).unwrap();
    assert!(wire["bytes"].is_string(), "internal source wire uses bounded base64, not a JSON integer array");
    let restored: nomi_native_files::NativeSource = serde_json::from_value(wire).unwrap();
    assert_eq!(restored.bytes, bytes);
    assert!(acquire_source_path(&source, bytes.len() - 1).is_err());
    assert!(acquire_source_path(&source, nomi_native_files::MAX_SOURCE_BYTES + 1).is_err());
    assert!(acquire(NativeReadRequest::Source { path: "relative.txt".into(), max_bytes: 32 }).is_err());
}

#[test]
fn source_absence_is_distinct_from_dangling_links_and_invalid_ancestors() {
    let fixture = tempfile::tempdir().unwrap();
    assert!(acquire_source_path(&fixture.path().join("absent/nested/file"), 1024).unwrap().is_none());
    let deep = fixture.path().join((0..130).map(|_| "x").collect::<Vec<_>>().join("/"));
    assert!(acquire_source_path(&deep, 1024).unwrap().is_none(), "source absence does not inherit the narrower instruction traversal budget");
    fs::write(fixture.path().join("not-a-directory"), b"data").unwrap();
    assert!(acquire_source_path(&fixture.path().join("not-a-directory/child"), 1024).is_err());
    #[cfg(unix)] {
        std::os::unix::fs::symlink("missing-target", fixture.path().join("dangling")).unwrap();
        assert!(matches!(acquire_source_path(&fixture.path().join("dangling"), 1024), Err(NativeFileError::Io { .. })));
    }
}

#[test]
fn a_changed_source_is_rejected_and_actual_read_bytes_still_charge_the_shared_budget() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"before").unwrap();
    let mut charged = 9;
    let result = acquire_source_with_adapter(1024, &mut charged,
        || fs::File::open(&source).map_err(|error| NativeFileError::io("fixture open", error)),
        || { fs::rename(&source, fixture.path().join("original")).unwrap(); fs::write(&source, b"after!").unwrap(); },
        || Ok(true), |error| error);
    assert!(matches!(result, Err(NativeFileError::Changed(_))));
    assert_eq!(charged, 15, "failed stable reads charge their actual source bytes");
}

#[cfg(unix)]
#[test]
fn explicit_links_resolve_but_changed_aliases_and_descendant_instruction_links_are_not_adopted() {
    use std::os::unix::fs::symlink;
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    fs::create_dir_all(root.join("directory/.hidden/deep")).unwrap();
    fs::create_dir(root.join("outside")).unwrap();
    fs::write(root.join("directory/AGENTS.md"), b"root rules").unwrap();
    fs::write(root.join("directory/.hidden/deep/AGENTS.override.md"), b"nested rules").unwrap();
    fs::write(root.join("outside/AGENTS.md"), b"outside rules").unwrap();
    symlink(root.join("outside"), root.join("directory/linked-directory")).unwrap();
    let scope = acquire_instruction_scope(&root.join("directory"), true).unwrap();
    assert_eq!(scope.kind, "directory");
    assert!(!scope.complete);
    assert_eq!(scope.incomplete_reasons, BTreeSet::from(["symlink_entry".into()]));
    assert_eq!(scope.directories, BTreeSet::from([
        root.join("directory").to_str().unwrap().into(), root.join("directory/.hidden/deep").to_str().unwrap().into(),
    ]));
    let explicit = acquire(NativeReadRequest::InstructionScope { path: root.join("directory/linked-directory").to_str().unwrap().into(), recursive: false, path_semantics: Default::default() }).unwrap();
    let NativeReadAcquisition::InstructionScope { scope } = explicit else { panic!("typed scope result"); };
    assert_eq!(Path::new(&scope.canonical_path), root.join("outside"));
    fs::write(root.join("first"), b"first").unwrap(); fs::write(root.join("second"), b"second").unwrap();
    let alias = root.join("alias"); symlink(root.join("first"), &alias).unwrap();
    let changed = acquire_source_with_hooks(&alias, 1024, || {}, || {
        fs::remove_file(&alias).unwrap(); symlink(root.join("second"), &alias).unwrap();
    });
    assert!(matches!(changed, Err(NativeFileError::Changed(_))));
}

#[cfg(unix)]
#[test]
fn denied_scope_and_source_are_native_access_errors_never_absence() {
    use std::os::unix::fs::PermissionsExt;
    struct Restore(std::path::PathBuf, fs::Permissions);
    impl Drop for Restore { fn drop(&mut self) { fs::set_permissions(&self.0, self.1.clone()).unwrap(); } }
    let fixture = tempfile::tempdir().unwrap();
    let parent = fixture.path().join("denied");
    let child = parent.join("child"); fs::create_dir_all(&child).unwrap();
    let source = child.join("AGENTS.md"); fs::write(&source, b"rules").unwrap();
    let restore = Restore(parent.clone(), fs::metadata(&parent).unwrap().permissions());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();
    let os_error = fs::canonicalize(&child).expect_err("native denial fixture requires an ordinary login user");
    assert_eq!(os_error.kind(), std::io::ErrorKind::PermissionDenied);
    for error in [acquire_source_path(&source, 1024).unwrap_err(), acquire_instruction_scope(&child, false).unwrap_err()] {
        let observed = error.observation();
        assert_eq!(observed.code, "SYSTEM_ACCESS_DENIED");
        assert_eq!(observed.raw_os_error, os_error.raw_os_error());
        assert!(std::error::Error::source(&error).is_some());
    }
    drop(restore);
}
