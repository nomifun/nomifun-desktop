#[path = "../src/backup.rs"]
mod backup;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use backup::{
    PluginBackupExport, PluginBackupFilesystem, PluginGrantMetadata, PluginPackageExport,
    PluginTransferError,
};
use nomifun_agent_contracts::{PLUGIN_MANIFEST_PATH, PluginId};
use nomifun_plugin_platform::{ArtifactStoreLimits, NeverCancel, PluginArtifactStore};
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

fn package_files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            PLUGIN_MANIFEST_PATH.into(),
            serde_json::to_vec_pretty(&json!({
                "schema": "nomifun.plugin/v1",
                "id": "local.backup-fixture",
                "version": "1.0.0",
                "name": "Backup fixture",
                "description": "Unified Plugin transfer fixture.",
                "hostApi": ">=1 <2",
                "entrypoints": {
                    "ui": "ui/index.html",
                    "service": "service/main.mjs",
                    "serviceMode": "onDemand"
                },
                "actions": {
                    "save": {
                        "name": "Save",
                        "description": "Save one item.",
                        "input": {"type": "object"},
                        "output": {"type": "object"},
                        "effect": "write"
                    }
                },
                "bindings": [{"point": "agent.tool", "action": "save"}],
                "dataVersion": 1,
                "migrations": [],
                "configSchema": {"type": "object"},
                "secrets": ["api_key"],
                "permissions": ["network"]
            }))
            .unwrap(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>Backup fixture</title>".to_vec(),
        ),
        (
            "service/main.mjs".into(),
            b"export async function activate(){return {async invoke(action,input){return {action,input}}}}".to_vec(),
        ),
        (
            "source/main.ts".into(),
            b"export const source = true;".to_vec(),
        ),
    ])
}

fn write_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (relative, bytes) in files {
        let path = relative
            .split('/')
            .fold(root.to_path_buf(), |path, component| path.join(component));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

struct Fixture {
    _temp: TempDir,
    transfer: PluginBackupFilesystem,
    artifact_root: PathBuf,
    artifact_digest: nomifun_agent_contracts::DigestHex,
    generation_root: PathBuf,
    output_root: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package-source");
    write_files(&package, &package_files());
    let artifact_store = PluginArtifactStore::new(
        temp.path().join("artifacts-a"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let imported = artifact_store
        .import_directory(&package, &NeverCancel)
        .unwrap();
    let artifact_root = imported.stored.package_root.clone();
    let artifact_digest = imported.stored.artifact.artifact_digest.clone();

    let generation_root = temp.path().join("generation-current");
    fs::create_dir_all(generation_root.join("files/notes")).unwrap();
    let database_path = generation_root.join("data.sqlite");
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE todos(id INTEGER PRIMARY KEY, title TEXT NOT NULL);\
             INSERT INTO todos(title) VALUES ('persisted');",
        )
        .unwrap();
    drop(connection);
    fs::write(generation_root.join("files/notes/todo.txt"), b"persisted file").unwrap();

    let transfer = PluginBackupFilesystem::new(temp.path().join("transfer-staging")).unwrap();
    let output_root = temp.path().join("outputs");
    fs::create_dir(&output_root).unwrap();
    Fixture {
        _temp: temp,
        transfer,
        artifact_root,
        artifact_digest,
        generation_root,
        output_root,
    }
}

fn package_request(fixture: &Fixture, include_source: bool) -> PluginPackageExport<'_> {
    PluginPackageExport {
        artifact_digest: &fixture.artifact_digest,
        package_root: &fixture.artifact_root,
        include_source,
    }
}

fn grants() -> Vec<PluginGrantMetadata> {
    vec![
        PluginGrantMetadata {
            permission: "network".into(),
            granted: false,
        },
        PluginGrantMetadata {
            permission: "desktop.files.open".into(),
            granted: true,
        },
    ]
}

fn backup_request<'a>(
    fixture: &'a Fixture,
    plugin_id: &'a PluginId,
    config: &'a serde_json::Value,
    grants: &'a [PluginGrantMetadata],
    slots: &'a BTreeSet<String>,
) -> PluginBackupExport<'a> {
    PluginBackupExport {
        plugin_id,
        artifact_digest: &fixture.artifact_digest,
        generation: "generation-1",
        package_root: &fixture.artifact_root,
        generation_root: &fixture.generation_root,
        config,
        grants,
        credential_slots: slots,
    }
}

fn relative_files(root: &Path) -> Vec<String> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn package_export_is_artifact_store_input_and_never_contains_user_data() {
    let fixture = fixture();
    let without_source = fixture.output_root.join("package-without-source");
    let stripped = fixture
        .transfer
        .export_package_directory(package_request(&fixture, false), &without_source)
        .unwrap();
    let paths = relative_files(&without_source);
    assert!(paths.contains(&PLUGIN_MANIFEST_PATH.to_owned()));
    assert!(!paths.iter().any(|path| path.starts_with("source/")));
    assert!(!paths.iter().any(|path| {
        path.starts_with("data/")
            || matches!(path.as_str(), "config.json" | "grants.json" | "credential-slots.json")
    }));
    let imported = fixture
        .transfer
        .import_package_directory(&without_source)
        .unwrap();
    assert_eq!(stripped, imported.artifact);
    let second_store = PluginArtifactStore::new(
        fixture.output_root.join("artifacts-b"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    assert_eq!(
        second_store
            .import_files(&imported.files, &NeverCancel)
            .unwrap()
            .stored
            .artifact,
        imported.artifact,
    );

    let archive = fixture.output_root.join("package-with-source.zip");
    let complete = fixture
        .transfer
        .export_package_zip(package_request(&fixture, true), &archive)
        .unwrap();
    assert_eq!(complete.artifact_digest, fixture.artifact_digest);
    let imported_zip = fixture.transfer.import_package_zip(&archive).unwrap();
    assert_eq!(imported_zip.artifact, complete);
    assert!(imported_zip.files.contains_key("source/main.ts"));
}

#[test]
fn backup_contains_only_current_data_and_non_secret_metadata() {
    let fixture = fixture();
    let plugin_id = PluginId::from("019b0000-0000-7000-8000-000000000123");
    let config = json!({"theme": "dark", "pageSize": 20});
    let grants = grants();
    let slots = BTreeSet::from(["api_key".to_owned()]);
    let destination = fixture.output_root.join("backup");
    let descriptor = fixture
        .transfer
        .export_backup_directory(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &destination,
        )
        .unwrap();
    assert_eq!(descriptor.plugin_id, plugin_id);
    assert_eq!(descriptor.artifact_digest, fixture.artifact_digest);
    assert_eq!(descriptor.generation, "generation-1");

    let paths = relative_files(&destination);
    for required in [
        "nomifun.plugin-backup.json",
        "package/nomifun.plugin.json",
        "data/data.sqlite",
        "data/files/notes/todo.txt",
        "config.json",
        "grants.json",
        "credential-slots.json",
    ] {
        assert!(paths.contains(&required.to_owned()), "missing {required}");
    }
    assert!(!paths.iter().any(|path| {
        path.starts_with("cache/")
            || path.starts_with("preview/")
            || path.contains("previous")
            || path.contains("credential_id")
    }));
    let all_bytes = paths
        .iter()
        .flat_map(|path| fs::read(destination.join(path)).unwrap())
        .collect::<Vec<_>>();
    let rendered = String::from_utf8_lossy(&all_bytes);
    assert!(!rendered.contains("credential-019b-secret"));
    assert!(!rendered.contains("plaintext-secret-value"));

    let imported = fixture.transfer.import_backup_directory(&destination).unwrap();
    assert_eq!(imported.plugin_id, plugin_id);
    assert_eq!(imported.artifact_digest, fixture.artifact_digest);
    assert_eq!(imported.generation, "generation-1");
    assert_eq!(imported.config, config);
    assert_eq!(imported.grants, {
        let mut expected = grants.clone();
        expected.sort();
        expected
    });
    assert_eq!(imported.credential_slots, slots);
    assert_eq!(imported.files["notes/todo.txt"], b"persisted file");
    assert!(imported.data_sqlite.starts_with(b"SQLite format 3\0"));
    assert_eq!(imported.package.artifact.artifact_digest, fixture.artifact_digest);

    let restored_artifacts = PluginArtifactStore::new(
        fixture.output_root.join("artifacts-restored"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    assert_eq!(
        restored_artifacts
            .import_files(&imported.package.files, &NeverCancel)
            .unwrap()
            .stored
            .artifact
            .artifact_digest,
        fixture.artifact_digest,
    );

    let zip = fixture.output_root.join("backup.zip");
    fixture
        .transfer
        .export_backup_zip(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &zip,
        )
        .unwrap();
    assert_eq!(
        fixture.transfer.import_backup_zip(zip).unwrap(),
        imported,
    );
}

#[test]
fn backup_rejects_secret_config_and_has_no_credential_binding_input() {
    let fixture = fixture();
    let plugin_id = PluginId::from("019b0000-0000-7000-8000-000000000124");
    let config = json!({
        "theme": "dark",
        "credential_id": "credential-019b-secret",
        "api_key": "plaintext-secret-value"
    });
    let grants = grants();
    let slots = BTreeSet::from(["api_key".to_owned()]);
    let destination = fixture.output_root.join("must-not-exist");
    assert!(matches!(
        fixture.transfer.export_backup_directory(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &destination,
        ),
        Err(PluginTransferError::InvalidInput(_))
    ));
    assert!(!destination.exists());
}

fn write_zip_from_directory(root: &Path, zip_path: &Path, extra: Option<&str>) {
    let file = fs::File::create(zip_path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o100600);
    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|component| component.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        writer.start_file(relative, options).unwrap();
        writer.write_all(&fs::read(entry.path()).unwrap()).unwrap();
    }
    if let Some(extra) = extra {
        writer.start_file(extra, options).unwrap();
        writer.write_all(b"escape").unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn tamper_traversal_and_zip_symlinks_are_rejected() {
    let fixture = fixture();
    let plugin_id = PluginId::from("019b0000-0000-7000-8000-000000000125");
    let config = json!({"theme": "dark"});
    let grants = grants();
    let slots = BTreeSet::from(["api_key".to_owned()]);
    let destination = fixture.output_root.join("backup-tamper");
    fixture
        .transfer
        .export_backup_directory(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &destination,
        )
        .unwrap();
    fs::write(destination.join("config.json"), br#"{"theme":"light"}"#).unwrap();
    assert!(matches!(
        fixture.transfer.import_backup_directory(&destination),
        Err(PluginTransferError::Tampered(_))
    ));

    let clean = fixture.output_root.join("backup-clean");
    fixture
        .transfer
        .export_backup_directory(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &clean,
        )
        .unwrap();
    let traversal = fixture.output_root.join("traversal.zip");
    write_zip_from_directory(&clean, &traversal, Some("../escape.txt"));
    assert!(matches!(
        fixture.transfer.import_backup_zip(&traversal),
        Err(PluginTransferError::UnsafePath { .. })
    ));

    let symlink = fixture.output_root.join("symlink.zip");
    write_zip_from_directory(&clean, &symlink, None);
    // Rebuild with a symbolic-link member because ZIP append is deliberately
    // unsupported by the writer.
    let file = fs::File::create(&symlink).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for entry in WalkDir::new(&clean).min_depth(1) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&clean)
            .unwrap()
            .components()
            .map(|component| component.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        writer
            .start_file(relative, SimpleFileOptions::default().unix_permissions(0o100600))
            .unwrap();
        writer.write_all(&fs::read(entry.path()).unwrap()).unwrap();
    }
    writer
        .add_symlink(
            "data/files/leak",
            "../../../outside",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.finish().unwrap();
    assert!(matches!(
        fixture.transfer.import_backup_zip(&symlink),
        Err(PluginTransferError::UnsafePath { .. })
    ));
}

#[test]
fn exports_atomically_reject_existing_destinations_without_overwrite() {
    let fixture = fixture();
    let existing_directory = fixture.output_root.join("existing-package");
    fs::create_dir(&existing_directory).unwrap();
    fs::write(existing_directory.join("sentinel"), b"keep").unwrap();
    assert!(matches!(
        fixture.transfer.export_package_directory(
            package_request(&fixture, true),
            &existing_directory,
        ),
        Err(PluginTransferError::DestinationExists(_))
    ));
    assert_eq!(fs::read(existing_directory.join("sentinel")).unwrap(), b"keep");

    let existing_zip = fixture.output_root.join("existing.zip");
    fs::write(&existing_zip, b"keep zip").unwrap();
    let plugin_id = PluginId::from("019b0000-0000-7000-8000-000000000126");
    let config = json!({"theme": "dark"});
    let grants = grants();
    let slots = BTreeSet::from(["api_key".to_owned()]);
    assert!(matches!(
        fixture.transfer.export_backup_zip(
            backup_request(&fixture, &plugin_id, &config, &grants, &slots),
            &existing_zip,
        ),
        Err(PluginTransferError::DestinationExists(_))
    ));
    assert_eq!(fs::read(&existing_zip).unwrap(), b"keep zip");
    assert!(fs::read_dir(&fixture.output_root)
        .unwrap()
        .map(Result::unwrap)
        .all(|entry| !entry.file_name().to_string_lossy().starts_with(".nomifun-plugin-transfer-")));
    assert_eq!(fs::read_dir(fixture.transfer.staging_root()).unwrap().count(), 0);
}
