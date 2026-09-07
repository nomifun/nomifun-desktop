use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, CapabilityActionDescriptor, CapabilityConsumer,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
    CanonicalSchemaRef, CredentialSlotDeclaration, CredentialSlotKey, CredentialSlotKind,
    DigestHex, EffectClass, ExactVersionRef, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile, JavaScriptEntrypointMetadata,
    LocalizedMetadata, MINIMUM_NODE_MAJOR, PLUGIN_N1_SCHEMA_VERSION,
    PLUGIN_PACKAGE_PROFILE_VERSION, PackageContributions, PackageId, PackageManifest,
    PlatformConstraint, PluginPackageV1Manifest, RuntimeTarget, StrictJsonValue,
    ToolPresentationKind, VersionString, canonical_json_bytes, capability_surface_declarations,
};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, ImportCancellation, NeverCancel, PluginArtifactStore,
    PluginArtifactStoreError,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

fn sha256(bytes: &[u8]) -> DigestHex {
    DigestHex::from(hex::encode(Sha256::digest(bytes)))
}

fn manifest(main: &[u8]) -> ArtifactEnvelope<PluginPackageV1Manifest> {
    let package = ExactVersionRef {
        id: PackageId::from("example.csv"),
        version: VersionString::from("1.0.0"),
    };
    let input_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
        "type": "object"
    }));
    let output_schema = StrictJsonValue(json!({
        "additionalProperties": true,
        "type": "object"
    }));
    let input_ref = CanonicalSchemaRef::from(format!(
        "schema://example/input@1#{}",
        nomifun_agent_contracts::digest_payload(&input_schema.0)
            .unwrap()
            .as_ref()
    ));
    let output_ref = CanonicalSchemaRef::from(format!(
        "schema://example/output@1#{}",
        nomifun_agent_contracts::digest_payload(&output_schema.0)
            .unwrap()
            .as_ref()
    ));
    let capability = CapabilityManifest {
        id: CapabilityId::from("example.csv.read"),
        contribution_id: "capability:example.csv.read".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: LocalizedMetadata {
            name: "CSV Read".into(),
            description: "Read CSV resources.".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type": "object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("example.csv.read.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::ReadLocal,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    ArtifactEnvelope::new(PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            package_id: package.id,
            package_version: package.version,
            display: LocalizedMetadata {
                name: "CSV Plugin".into(),
                description: "CSV capability package.".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: JavaScriptEntrypointMetadata {
                normalized_relative_path: "main.mjs".into(),
                module_digest: sha256(main),
                host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            }
            .into(),
            contributions: PackageContributions {
                capabilities: vec![capability],
                ..Default::default()
            },
        },
        schemas: BTreeMap::from([
            (input_ref, input_schema),
            (output_ref, output_schema),
        ]),
        supported_targets: BTreeSet::from([RuntimeTarget::from(
            "x86_64-pc-windows-msvc",
        )]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: DigestHex::from("b".repeat(64)),
        credential_slots: vec![CredentialSlotDeclaration {
            slot_key: CredentialSlotKey::from("api_key"),
            kind: CredentialSlotKind::SecretText,
            display_name: "API key".into(),
            required: false,
        }],
    })
    .unwrap()
}

fn package_directory(root: &Path, main: &[u8]) {
    fs::create_dir_all(root.join("resources/nested")).unwrap();
    fs::write(
        root.join("manifest.json"),
        canonical_json_bytes(&manifest(main)).unwrap(),
    )
    .unwrap();
    fs::write(root.join("main.mjs"), main).unwrap();
    fs::write(root.join("main.mjs.map"), b"{}").unwrap();
    fs::write(root.join("resources/nested/schema.json"), b"{}").unwrap();
}

fn package_zip(path: &Path, main: &[u8], extra: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o100600);
    let manifest_bytes = canonical_json_bytes(&manifest(main)).unwrap();
    for (name, bytes) in [
        ("manifest.json", manifest_bytes.as_slice()),
        ("main.mjs", main),
        ("main.mjs.map", b"{}".as_slice()),
        ("resources/nested/schema.json", b"{}".as_slice()),
    ]
    .into_iter()
    .chain(extra.iter().copied())
    {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn fixture() -> (TempDir, PluginArtifactStore, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        PluginArtifactStore::new(temp.path().join("managed"), ArtifactStoreLimits::default())
            .unwrap();
    let package = temp.path().join("package");
    package_directory(&package, b"export const plugin = 1;\n");
    (temp, store, package)
}

#[test]
fn directory_and_zip_import_share_one_idempotent_artifact() {
    let (temp, store, package) = fixture();
    let first = store.import_directory(&package, &NeverCancel).unwrap();
    assert!(!first.already_present);
    assert!(uuid::Uuid::parse_str(first.stored.artifact.artifact_id.as_ref()).is_ok());
    assert_eq!(
        first.stored.managed_relative_path,
        format!(
            "artifacts/{}",
            first.stored.artifact.artifact_digest.as_ref()
        )
    );

    let second = store.import_directory(&package, &NeverCancel).unwrap();
    assert!(second.already_present);
    assert_eq!(
        first.stored.artifact.artifact_id,
        second.stored.artifact.artifact_id
    );

    let archive = temp.path().join("package.zip");
    package_zip(&archive, b"export const plugin = 1;\n", &[]);
    let third = store.import_zip(&archive, &NeverCancel).unwrap();
    assert!(third.already_present);
    assert_eq!(first.stored.artifact, third.stored.artifact);
    assert_eq!(
        fs::read_dir(store.managed_root().join("artifacts"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn published_tamper_is_rejected_without_replacement() {
    let (_temp, store, package) = fixture();
    let imported = store.import_directory(&package, &NeverCancel).unwrap();
    let entrypoint = imported.stored.package_root.join("main.mjs");
    fs::write(&entrypoint, b"tampered").unwrap();

    let error = store.import_directory(&package, &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        PluginArtifactStoreError::PublishedArtifactMismatch { .. }
    ));
    assert_eq!(fs::read(entrypoint).unwrap(), b"tampered");
}

#[test]
fn published_root_rejects_untracked_content() {
    let (_temp, store, package) = fixture();
    let imported = store.import_directory(&package, &NeverCancel).unwrap();
    fs::write(imported.stored.artifact_root.join("untracked.txt"), b"x").unwrap();
    assert!(matches!(
        store.load(&imported.stored.artifact.artifact_digest),
        Err(PluginArtifactStoreError::PublishedArtifactMismatch { .. })
    ));
}

#[test]
fn zip_rejects_traversal_backslash_and_case_collision() {
    let temp = tempfile::tempdir().unwrap();
    let store =
        PluginArtifactStore::new(temp.path().join("managed"), ArtifactStoreLimits::default())
            .unwrap();
    for (name, extra_name) in [
        ("traversal.zip", "../escape.txt"),
        ("backslash.zip", "resources\\escape.txt"),
        ("absolute.zip", "/absolute.txt"),
    ] {
        let archive = temp.path().join(name);
        package_zip(&archive, b"export const plugin = 1;\n", &[(extra_name, b"x")]);
        assert!(matches!(
            store.import_zip(&archive, &NeverCancel),
            Err(PluginArtifactStoreError::UnsafePackagePath { .. })
        ));
    }

    let collision = temp.path().join("collision.zip");
    package_zip(
        &collision,
        b"export const plugin = 1;\n",
        &[("resources/A.txt", b"a"), ("resources/a.txt", b"b")],
    );
    assert!(matches!(
        store.import_zip(&collision, &NeverCancel),
        Err(PluginArtifactStoreError::DuplicateEntry { .. })
    ));

    let symlink = temp.path().join("symlink.zip");
    let file = fs::File::create(&symlink).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
        .add_symlink(
            "resources/link",
            "../../outside",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.finish().unwrap();
    assert!(matches!(
        store.import_zip(&symlink, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafePackagePath { .. })
    ));
}

#[test]
fn manifest_requires_one_verified_envelope() {
    let (_temp, store, package) = fixture();
    let original = fs::read_to_string(package.join("manifest.json")).unwrap();
    let duplicate = original.replacen(
        "{\"digest_algorithm\":",
        "{\"digest_algorithm\":\"wrong\",\"digest_algorithm\":",
        1,
    );
    fs::write(package.join("manifest.json"), duplicate).unwrap();
    let error = store.import_directory(&package, &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        PluginArtifactStoreError::InvalidManifest(message)
            if message.contains("duplicate decoded JSON object key")
    ));
}

struct CancelAfter {
    checks: AtomicUsize,
    allowed_checks: usize,
}

impl ImportCancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::AcqRel) >= self.allowed_checks
    }
}

#[test]
fn cancellation_removes_only_its_unique_staging_directory() {
    let (_temp, store, package) = fixture();
    let cancellation = CancelAfter {
        checks: AtomicUsize::new(0),
        allowed_checks: 4,
    };
    assert!(matches!(
        store.import_directory(&package, &cancellation),
        Err(PluginArtifactStoreError::Canceled)
    ));
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(store.managed_root().join("artifacts"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn package_root_rejects_unrelated_files_and_size_overflow() {
    let (_temp, store, package) = fixture();
    fs::write(package.join("package.json"), b"{}").unwrap();
    assert!(matches!(
        store.import_directory(&package, &NeverCancel),
        Err(PluginArtifactStoreError::UnsupportedEntry { .. })
    ));

    fs::remove_file(package.join("package.json")).unwrap();
    let tiny = PluginArtifactStore::new(
        store.managed_root().parent().unwrap().join("tiny-managed"),
        ArtifactStoreLimits {
            max_file_count: 8,
            max_manifest_bytes: 1024 * 1024,
            max_single_file_bytes: 8,
            max_total_bytes: 1024 * 1024,
            max_zip_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    assert!(matches!(
        tiny.import_directory(&package, &NeverCancel),
        Err(PluginArtifactStoreError::FileTooLarge { .. })
    ));
}

#[test]
fn directory_source_cannot_contain_or_be_contained_by_managed_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    package_directory(&source, b"export const plugin = 1;\n");
    let nested_store = PluginArtifactStore::new(
        source.join("managed"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        nested_store.import_directory(&source, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafeManagedPath { .. })
    ));

    let outer_store =
        PluginArtifactStore::new(temp.path().join("managed"), ArtifactStoreLimits::default())
            .unwrap();
    let nested_source = outer_store.managed_root().join("incoming");
    package_directory(&nested_source, b"export const plugin = 1;\n");
    assert!(matches!(
        outer_store.import_directory(&nested_source, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafeManagedPath { .. })
    ));
}

#[test]
fn directory_rejects_non_nfc_portable_paths() {
    let (_temp, store, package) = fixture();
    let decomposed = "cafe\u{301}.txt";
    fs::write(package.join("resources").join(decomposed), b"x").unwrap();
    assert!(matches!(
        store.import_directory(&package, &NeverCancel),
        Err(PluginArtifactStoreError::UnsafePackagePath { reason, .. })
            if reason.contains("NFC")
    ));
}
