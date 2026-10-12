//! Offline registry proof is read-only; no HostingOwner, boot recovery or
//! running program is initialized for these storage fixtures.
use std::{collections::BTreeSet, fs, path::{Path, PathBuf}};
use nomifun_app_hosting::{AppRecord, AppSettings, AppState, ImportKind,
    inspect_offline_hosted_apps, rewrite_offline_hosted_app_registries};
use serde_json::{Value, json};

struct Fixture {
    _temporary: tempfile::TempDir,
    source: PathBuf,
    staging: PathBuf,
    target: PathBuf,
    id: String,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(temporary.path()).unwrap();
        let source = base.join("source");
        let staging = base.join("staging");
        let target = base.join("final-destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&staging).unwrap();
        let mut fixture = Self { _temporary: temporary, source, staging, target, id: uuid::Uuid::now_v7().to_string() };
        fixture.write_record(fixture.id.clone());
        fixture
    }

    fn write_record(&mut self, id: String) {
        let directory = self.source.join("app-hosting").join("apps").join(&id);
        fs::create_dir_all(&directory).unwrap();
        fs::create_dir_all(self.source.join("app-hosting").join("registry")).unwrap();
        fs::write(directory.join("user-work.txt"), b"EXACT_USER_SOURCE").unwrap();
        let record = AppRecord { id: id.clone(), name: "独立应用项目".into(), description: "保留配置和源码".into(),
            tags: vec!["工具".into()], development_session_id: Some(uuid::Uuid::now_v7().to_string()), directory,
            import_kind: ImportKind::Source, settings: AppSettings { preview_command: Some("node server.cjs".into()),
                start_command: Some("node server.cjs".into()), ..Default::default() }, state: AppState::Stopped,
            operation: None, last_error: Some("preserved diagnostic".into()), output: "preserved command output".into(),
            pid: None, preview: true, prepared: true };
        fs::write(self.registry(&id), serde_json::to_vec_pretty(&json!({"record":record,"identity":null})).unwrap()).unwrap();
    }

    fn registry(&self, id: &str) -> PathBuf { self.source.join("app-hosting").join("registry").join(format!("{id}.json")) }
    fn staged_registry(&self, id: &str) -> PathBuf { self.staging.join("app-hosting").join("registry").join(format!("{id}.json")) }
    fn read(&self) -> Value { serde_json::from_slice(&fs::read(self.registry(&self.id)).unwrap()).unwrap() }
    fn edit(&self, edit: impl FnOnce(&mut Value)) {
        let mut value = self.read();
        edit(&mut value);
        fs::write(self.registry(&self.id), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    fn copy(&self) { copy_tree(&self.source, &self.staging); }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() { copy_tree(&entry.path(), &target.join(entry.file_name())); }
        else { fs::copy(entry.path(), target.join(entry.file_name())).unwrap(); }
    }
}

#[test]
fn offline_relocation_rebases_only_copied_directory_and_preserves_source_and_all_other_values() {
    let mut fixture = Fixture::new();
    let second = uuid::Uuid::now_v7().to_string();
    fixture.write_record(second.clone());
    // Accepted empty current-generation retirement metadata must not be
    // normalized or dropped as a side effect of moving the storage location.
    fixture.edit(|value| value["record"]["settings"]["exclusive_session_ids"] = json!([]));
    let source_bytes = fs::read(fixture.registry(&fixture.id)).unwrap();
    let second_bytes = fs::read(fixture.registry(&second)).unwrap();
    let original = fixture.read();
    let inventory = inspect_offline_hosted_apps(&fixture.source).unwrap();
    let ids = BTreeSet::from([fixture.id.clone(), second.clone()]);
    assert_eq!(inventory.app_ids, ids);
    assert_eq!(inventory.owned_directories, ids.iter().map(|id| fixture.source.join("app-hosting").join("apps").join(id)).collect());
    assert_eq!(fs::read(fixture.registry(&fixture.id)).unwrap(), source_bytes);
    assert!(!fixture.target.exists(), "offline inspection cannot materialize a destination");
    fixture.copy();
    assert_eq!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).unwrap(), ids);
    let mut expected = original;
    expected["record"]["directory"] = json!(fixture.target.join("app-hosting").join("apps").join(&fixture.id));
    let staged: Value = serde_json::from_slice(&fs::read(fixture.staged_registry(&fixture.id)).unwrap()).unwrap();
    assert_eq!(staged, expected);
    assert_eq!(fs::read(fixture.registry(&fixture.id)).unwrap(), source_bytes);
    assert_eq!(fs::read(fixture.registry(&second)).unwrap(), second_bytes);
    assert_eq!(fs::read(fixture.source.join("app-hosting").join("apps").join(&fixture.id).join("user-work.txt")).unwrap(), b"EXACT_USER_SOURCE");
    assert_eq!(fs::read(fixture.staging.join("app-hosting").join("apps").join(&fixture.id).join("user-work.txt")).unwrap(), b"EXACT_USER_SOURCE");
    let first_write = fs::read(fixture.staged_registry(&fixture.id)).unwrap();
    assert_eq!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).unwrap(), ids);
    assert_eq!(fs::read(fixture.staged_registry(&fixture.id)).unwrap(), first_write, "an exact retry is a no-op");
    assert!(!fixture.target.exists(), "this callback only rewrites unpublished staging files");
}

#[test]
fn offline_inspection_accepts_an_uninitialized_hosting_domain_without_creating_it() {
    let temporary = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(temporary.path()).unwrap();
    let source = base.join("source"); let staging = base.join("staging");
    fs::create_dir(&source).unwrap(); fs::create_dir(&staging).unwrap();
    assert!(inspect_offline_hosted_apps(&source).unwrap().app_ids.is_empty());
    assert!(rewrite_offline_hosted_app_registries(&source, &staging, &base.join("target")).unwrap().is_empty());
    assert!(!source.join("app-hosting").exists());
    assert!(!staging.join("app-hosting").exists());
}

#[test]
fn offline_inspection_rejects_process_identity_pid_running_and_in_progress_operations_without_recovery() {
    for field in ["identity", "pid", "state", "operation"] {
        let fixture = Fixture::new();
        fixture.edit(|value| match field {
            "identity" => value["identity"] = json!({"pid":12345,"start_time_epoch_seconds":1,"platform_start_key":1}),
            "pid" => value["record"]["pid"] = json!(12345),
            "state" => value["record"]["state"] = json!("running"),
            "operation" => value["record"]["operation"] = json!("installing"),
            _ => unreachable!(),
        });
        let before = fs::read(fixture.registry(&fixture.id)).unwrap();
        let error = inspect_offline_hosted_apps(&fixture.source).unwrap_err().to_string();
        assert!(error.contains("settled process identity"), "{field}: {error}");
        fixture.copy();
        let staged = fs::read(fixture.staged_registry(&fixture.id)).unwrap();
        assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).is_err());
        assert_eq!(fs::read(fixture.registry(&fixture.id)).unwrap(), before);
        assert_eq!(fs::read(fixture.staged_registry(&fixture.id)).unwrap(), staged);
        assert_eq!(fs::read(fixture.source.join("app-hosting").join("apps").join(&fixture.id).join("user-work.txt")).unwrap(), b"EXACT_USER_SOURCE");
    }
}

#[test]
fn offline_inspection_rejects_forged_names_ids_owner_directories_and_registry_identity() {
    for case in ["name", "id", "directory", "stem", "retired-claim", "unknown-field"] {
        let fixture = Fixture::new();
        match case {
            "name" => fixture.edit(|value| value["record"]["name"] = json!(" \t")),
            "id" => fixture.edit(|value| value["record"]["id"] = json!("550e8400-e29b-41d4-a716-446655440000")),
            "directory" => fixture.edit(|value| value["record"]["directory"] = json!(fixture.source.join("unrelated-project"))),
            "stem" => fs::rename(fixture.registry(&fixture.id), fixture.registry(&uuid::Uuid::now_v7().to_string())).unwrap(),
            "retired-claim" => fixture.edit(|value| value["record"]["settings"]["exclusive_session_ids"] = json!([uuid::Uuid::now_v7().to_string()])),
            "unknown-field" => fixture.edit(|value| value["record"]["alternate_directory"] = json!("unowned")),
            _ => unreachable!(),
        }
        let before = fs::read_dir(fixture.source.join("app-hosting").join("registry")).unwrap()
            .map(|entry| { let path = entry.unwrap().path(); (path.clone(), fs::read(path).unwrap()) }).collect::<Vec<_>>();
        assert!(inspect_offline_hosted_apps(&fixture.source).is_err(), "forged {case} must fail closed");
        for (path, bytes) in before { assert_eq!(fs::read(path).unwrap(), bytes); }
    }
}

#[test]
fn offline_relocation_validates_the_entire_copy_before_changing_any_staging_registry() {
    let mut fixture = Fixture::new();
    let second = uuid::Uuid::now_v7().to_string();
    fixture.write_record(second.clone());
    fixture.copy();
    let first_before = fs::read(fixture.staged_registry(&fixture.id)).unwrap();
    let path = fixture.staged_registry(&second);
    let mut tampered: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    tampered["record"]["name"] = json!("Copy was edited beyond relocation");
    fs::write(&path, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
    let second_before = fs::read(&path).unwrap();
    assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).is_err());
    assert_eq!(fs::read(fixture.staged_registry(&fixture.id)).unwrap(), first_before);
    assert_eq!(fs::read(&path).unwrap(), second_before);
    assert!(!fixture.target.exists());
}

#[test]
fn offline_relocation_rejects_different_or_missing_copied_registrations_and_source_as_destination() {
    for extra in [false, true] {
        let fixture = Fixture::new();
        fixture.copy();
        if extra {
            fs::copy(fixture.staged_registry(&fixture.id), fixture.staged_registry(&uuid::Uuid::now_v7().to_string())).unwrap();
        } else { fs::remove_file(fixture.staged_registry(&fixture.id)).unwrap(); }
        assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).is_err());
    }
    let fixture = Fixture::new();
    fixture.copy();
    let before = fs::read(fixture.registry(&fixture.id)).unwrap();
    assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.source, &fixture.target).is_err());
    assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.source).is_err());
    assert_eq!(fs::read(fixture.registry(&fixture.id)).unwrap(), before);
}

#[cfg(windows)]
mod windows_redirects {
    use super::*;
    struct Junction(PathBuf);
    impl Drop for Junction { fn drop(&mut self) { let _ = junction::delete(&self.0); } }

    #[test]
    fn offline_inspection_and_rewrite_refuse_redirected_source_staging_and_destination_roots() {
        for redirect in ["source-app", "source-registry", "staged-app", "staged-registry", "target"] {
            let fixture = Fixture::new();
            fixture.copy();
            let external = fixture.source.parent().unwrap().join("external-owner");
            fs::create_dir(&external).unwrap();
            fs::write(external.join("user-work.txt"), b"EXTERNAL_OWNER_DATA").unwrap();
            let redirected = match redirect {
                "source-app" => fixture.source.join("app-hosting").join("apps").join(&fixture.id),
                "source-registry" => fixture.source.join("app-hosting").join("registry"),
                "staged-app" => fixture.staging.join("app-hosting").join("apps").join(&fixture.id),
                "staged-registry" => fixture.staging.join("app-hosting").join("registry"),
                "target" => fixture.target.clone(),
                _ => unreachable!(),
            };
            if redirected.exists() { fs::rename(&redirected, redirected.with_extension("retained")).unwrap(); }
            junction::create(&external, &redirected).unwrap();
            let _junction = Junction(redirected);
            if redirect.starts_with("source") { assert!(inspect_offline_hosted_apps(&fixture.source).is_err()); }
            assert!(rewrite_offline_hosted_app_registries(&fixture.source, &fixture.staging, &fixture.target).is_err(), "{redirect}");
            assert_eq!(fs::read(external.join("user-work.txt")).unwrap(), b"EXTERNAL_OWNER_DATA");
        }
    }
}
