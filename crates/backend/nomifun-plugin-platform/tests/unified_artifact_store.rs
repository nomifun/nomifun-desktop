use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use nomifun_agent_contracts::{
    PLUGIN_MANIFEST_PATH, PluginManifest, canonical_json_bytes,
};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, NeverCancel, PluginArtifactStore, PluginArtifactStoreError,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

#[derive(Clone, Copy)]
enum Shape {
    UiOnly,
    Headless,
    Mixed,
}

fn manifest_value(id: &str, shape: Shape) -> Value {
    let (entrypoints, actions, bindings) = match shape {
        Shape::UiOnly => (
            json!({"ui": "ui/index.html"}),
            json!({}),
            json!([]),
        ),
        Shape::Headless => (
            json!({"service": "service/main.mjs", "serviceMode": "onDemand"}),
            json!({
                "ping": {
                    "name": "Ping",
                    "description": "Returns the supplied value.",
                    "input": {"type": "object"},
                    "output": {"type": "object"},
                    "effect": "read"
                }
            }),
            json!([{"point": "agent.tool", "action": "ping"}]),
        ),
        Shape::Mixed => (
            json!({
                "ui": "ui/index.html",
                "service": "service/main.mjs",
                "serviceMode": "continuous"
            }),
            json!({
                "save": {
                    "name": "Save",
                    "description": "Saves one item.",
                    "input": {
                        "type": "object",
                        "properties": {"value": {}},
                        "required": ["value"]
                    },
                    "output": {"type": "object"},
                    "effect": "write"
                }
            }),
            json!([
                {"point": "agent.tool", "action": "save"},
                {"point": "desktop.command", "action": "save"}
            ]),
        ),
    };
    json!({
        "schema": "nomifun.plugin/v1",
        "id": id,
        "version": "1.0.0",
        "name": format!("{id} fixture"),
        "description": "Unified Plugin Artifact Store fixture.",
        "hostApi": ">=1 <2",
        "entrypoints": entrypoints,
        "actions": actions,
        "bindings": bindings,
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type": "object"},
        "secrets": [],
        "permissions": []
    })
}

fn manifest_bytes(id: &str, shape: Shape) -> Vec<u8> {
    serde_json::to_vec_pretty(&manifest_value(id, shape)).unwrap()
}

fn package_files(id: &str, shape: Shape) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::from([(
        PLUGIN_MANIFEST_PATH.to_owned(),
        manifest_bytes(id, shape),
    )]);
    if matches!(shape, Shape::UiOnly | Shape::Mixed) {
        files.insert(
            "ui/index.html".into(),
            b"<!doctype html><title>Unified fixture</title>".to_vec(),
        );
        files.insert("ui/assets/app.js".into(), b"window.fixture = true;".to_vec());
    }
    if matches!(shape, Shape::Headless | Shape::Mixed) {
        files.insert(
            "service/main.mjs".into(),
            b"export async function activate(){return {async invoke(action,input){return {action,input}}}}".to_vec(),
        );
    }
    files
}

fn write_directory(root: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (relative, bytes) in files {
        let path = relative
            .split('/')
            .fold(root.to_path_buf(), |path, component| path.join(component));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

fn write_zip(path: &Path, files: &BTreeMap<String, Vec<u8>>) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o100600);
    for (relative, bytes) in files {
        writer.start_file(relative, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn store(temp: &TempDir) -> PluginArtifactStore {
    PluginArtifactStore::new(
        temp.path().join("managed"),
        ArtifactStoreLimits::default(),
    )
    .unwrap()
}

#[test]
fn bytes_directory_and_zip_share_one_canonical_artifact_digest() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let files = package_files("local.shared", Shape::Mixed);

    let from_bytes = store.import_files(&files, &NeverCancel).unwrap();
    assert!(!from_bytes.already_present);

    let directory = temp.path().join("directory-package");
    let mut directory_files = files.clone();
    directory_files.insert(
        PLUGIN_MANIFEST_PATH.into(),
        serde_json::to_vec(&manifest_value("local.shared", Shape::Mixed)).unwrap(),
    );
    write_directory(&directory, &directory_files);
    let from_directory = store.import_directory(&directory, &NeverCancel).unwrap();
    assert!(from_directory.already_present);

    let archive = temp.path().join("package.zip");
    let mut zip_files = files.clone();
    zip_files
        .get_mut(PLUGIN_MANIFEST_PATH)
        .unwrap()
        .extend_from_slice(b"\n");
    write_zip(&archive, &zip_files);
    let from_zip = store.import_zip(&archive, &NeverCancel).unwrap();
    assert!(from_zip.already_present);

    assert_eq!(
        from_bytes.stored.artifact.artifact_digest,
        from_directory.stored.artifact.artifact_digest,
    );
    assert_eq!(from_bytes.stored.artifact, from_zip.stored.artifact);
    assert_eq!(
        fs::read_dir(store.managed_root().join("artifacts"))
            .unwrap()
            .count(),
        1,
    );

    let stored_manifest = fs::read(from_bytes.stored.package_root.join(PLUGIN_MANIFEST_PATH))
        .unwrap();
    let parsed = PluginManifest::parse(&files[PLUGIN_MANIFEST_PATH]).unwrap();
    assert_eq!(stored_manifest, canonical_json_bytes(&parsed).unwrap());
}

#[test]
fn ui_only_headless_and_mixed_packages_are_first_class_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let fixtures = [
        ("local.ui", Shape::UiOnly, true, false),
        ("local.headless", Shape::Headless, false, true),
        ("local.mixed", Shape::Mixed, true, true),
    ];

    for (id, shape, has_ui, has_service) in fixtures {
        let imported = store
            .import_files(&package_files(id, shape), &NeverCancel)
            .unwrap();
        assert_eq!(imported.stored.artifact.manifest.has_ui(), has_ui);
        assert_eq!(imported.stored.artifact.manifest.has_service(), has_service);
        assert_eq!(imported.stored.artifact.manifest.id, id);
        assert_eq!(
            imported
                .stored
                .artifact
                .files
                .iter()
                .any(|file| file.normalized_relative_path == "ui/index.html"),
            has_ui,
        );
        assert_eq!(
            imported
                .stored
                .artifact
                .files
                .iter()
                .any(|file| file.normalized_relative_path == "service/main.mjs"),
            has_service,
        );
    }
}

#[test]
fn traversal_and_symbolic_links_are_rejected_for_all_import_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);

    let mut captured = package_files("local.capture", Shape::UiOnly);
    captured.insert("../escape.txt".into(), b"escape".to_vec());
    assert!(matches!(
        store.import_files(&captured, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafePackagePath { .. })
    ));

    let traversal = temp.path().join("traversal.zip");
    let file = fs::File::create(&traversal).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().unix_permissions(0o100600);
    for (relative, bytes) in package_files("local.zip", Shape::UiOnly) {
        writer.start_file(relative, options).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.start_file("../escape.txt", options).unwrap();
    writer.write_all(b"escape").unwrap();
    writer.finish().unwrap();
    assert!(matches!(
        store.import_zip(&traversal, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafePackagePath { .. })
    ));

    let symlink = temp.path().join("symlink.zip");
    let file = fs::File::create(&symlink).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for (relative, bytes) in package_files("local.link", Shape::UiOnly) {
        writer.start_file(relative, options).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer
        .add_symlink("ui/leak", "../../outside", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();
    assert!(matches!(
        store.import_zip(&symlink, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafePackagePath { .. })
    ));
}

#[test]
fn unknown_manifest_fields_and_legacy_manifest_envelopes_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);

    let mut unknown = package_files("local.unknown", Shape::UiOnly);
    let mut value = manifest_value("local.unknown", Shape::UiOnly);
    value
        .as_object_mut()
        .unwrap()
        .insert("legacy".into(), Value::Bool(true));
    unknown.insert(PLUGIN_MANIFEST_PATH.into(), serde_json::to_vec(&value).unwrap());
    assert!(matches!(
        store.import_files(&unknown, &NeverCancel),
        Err(PluginArtifactStoreError::InvalidManifest(_))
    ));

    let mut envelope = package_files("local.envelope", Shape::UiOnly);
    envelope.insert(
        PLUGIN_MANIFEST_PATH.into(),
        serde_json::to_vec(&json!({
            "digest_algorithm": "sorted-json-sha256-v1",
            "payload_digest": "a".repeat(64),
            "payload": manifest_value("local.envelope", Shape::UiOnly)
        }))
        .unwrap(),
    );
    assert!(matches!(
        store.import_files(&envelope, &NeverCancel),
        Err(PluginArtifactStoreError::InvalidManifest(_))
    ));

    let mut old_path = package_files("local.old-path", Shape::UiOnly);
    let manifest = old_path.remove(PLUGIN_MANIFEST_PATH).unwrap();
    old_path.insert("manifest.json".into(), manifest);
    assert!(matches!(
        store.import_files(&old_path, &NeverCancel),
        Err(PluginArtifactStoreError::UnsupportedEntry { .. })
    ));
}

#[test]
fn published_artifact_tamper_is_detected_without_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let files = package_files("local.tamper", Shape::Headless);
    let imported = store.import_files(&files, &NeverCancel).unwrap();
    let digest = imported.stored.artifact.artifact_digest.clone();
    let entrypoint = imported.stored.package_root.join("service/main.mjs");
    fs::write(&entrypoint, b"export const tampered = true;").unwrap();

    assert!(matches!(
        store.load(&digest),
        Err(PluginArtifactStoreError::PublishedArtifactMismatch { .. })
    ));
    assert!(matches!(
        store.import_files(&files, &NeverCancel),
        Err(PluginArtifactStoreError::PublishedArtifactMismatch { .. })
    ));
    assert_eq!(
        fs::read(entrypoint).unwrap(),
        b"export const tampered = true;",
    );
}
