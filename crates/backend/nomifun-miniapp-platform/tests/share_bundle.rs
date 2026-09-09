use std::collections::BTreeMap;
use std::fs;

use nomifun_agent_contracts::{
    digest_bytes, ArtifactId, LocalizedMetadata, MiniAppId, MiniAppResourceContract,
    MiniAppShareBundleId, PackageRef, StrictJsonValue,
};
use nomifun_miniapp_platform::{
    cleanup_miniapp_share_staging, materialize_surface_entrypoint, MiniAppReleaseFileBytes,
    MiniAppReleasePublishRequest, MiniAppReleaseStore, MiniAppShareBundleError,
    MiniAppShareBundleExport, MiniAppShareBundleFilesystem, MiniAppShareImportEntry,
    MiniAppShareSourceExport, MiniAppSourceFileInput, MiniAppSourceScope, MiniAppSourceStore,
    MiniAppStaticBundleBuilder, MiniAppStaticBundleFile, MiniAppStaticBundleInput,
};

#[test]
fn share_bundle_roundtrip_preserves_release_and_optional_source() {
    let fixture = Fixture::new();
    let destination = fixture.temp.path().join("notes.nomifun-miniapp");
    let filesystem = MiniAppShareBundleFilesystem::default();
    let bundle = filesystem
        .export(fixture.export_request(true), &destination)
        .unwrap();

    let imported = filesystem.import_share_bundle(&destination).unwrap();
    assert_eq!(imported.bundle, bundle);
    assert_eq!(imported.release.artifact, fixture.release.artifact);
    assert_eq!(imported.release.files, fixture.release.files);
    let source = imported.source.unwrap();
    assert_eq!(
        source.source.source_snapshot_digest,
        fixture.source.source_snapshot_digest
    );
    assert_eq!(source.dependency_lock, fixture.source.dependency_lock);
    assert_eq!(source.files, fixture.source.files);
    assert!(!destination.join("config.json").exists());
    assert!(!destination.join("storage").exists());

    assert!(matches!(
        filesystem.export(fixture.export_request(true), &destination),
        Err(MiniAppShareBundleError::DestinationExists(_))
    ));
    assert_eq!(
        cleanup_miniapp_share_staging(fixture.temp.path()).unwrap(),
        0
    );
}

#[test]
fn share_bundle_rejects_tamper_extra_missing_and_noncanonical_metadata() {
    let fixture = Fixture::new();
    let filesystem = MiniAppShareBundleFilesystem::default();

    let tampered = fixture.temp.path().join("tampered");
    filesystem
        .export(fixture.export_request(true), &tampered)
        .unwrap();
    fs::write(
        tampered.join("release/files/ui/index.html"),
        b"modified",
    )
    .unwrap();
    assert!(matches!(
        filesystem.import_share_bundle(&tampered),
        Err(MiniAppShareBundleError::Tampered(_))
    ));

    let extra = fixture.temp.path().join("extra");
    filesystem
        .export(fixture.export_request(true), &extra)
        .unwrap();
    fs::write(extra.join("credential.json"), b"{}").unwrap();
    assert!(matches!(
        filesystem.import_share_bundle(&extra),
        Err(MiniAppShareBundleError::InvalidInventory(_))
    ));

    let missing = fixture.temp.path().join("missing");
    filesystem
        .export(fixture.export_request(true), &missing)
        .unwrap();
    fs::remove_file(missing.join("source/dependency-lock.json")).unwrap();
    assert!(filesystem.import_share_bundle(&missing).is_err());

    let noncanonical = fixture.temp.path().join("noncanonical");
    filesystem
        .export(fixture.export_request(true), &noncanonical)
        .unwrap();
    let bundle_path = noncanonical.join("bundle.json");
    let mut bytes = fs::read(&bundle_path).unwrap();
    bytes.push(b'\n');
    fs::write(bundle_path, bytes).unwrap();
    assert!(matches!(
        filesystem.import_share_bundle(&noncanonical),
        Err(MiniAppShareBundleError::Tampered(_))
    ));
}

#[test]
fn source_less_bundle_and_prebuilt_release_share_the_release_validator() {
    let fixture = Fixture::new();
    let filesystem = MiniAppShareBundleFilesystem::default();
    let source_less = fixture.temp.path().join("runtime-only");
    filesystem
        .export(fixture.export_request(false), &source_less)
        .unwrap();
    let imported = filesystem.import_share_bundle(&source_less).unwrap();
    assert!(imported.source.is_none());

    let prebuilt = fixture.temp.path().join("prebuilt");
    fs::create_dir(&prebuilt).unwrap();
    copy_tree(&source_less.join("release"), &prebuilt);
    let imported = filesystem.import_prebuilt_release(&prebuilt).unwrap();
    assert_eq!(imported.artifact, fixture.release.artifact);
    assert!(matches!(
        filesystem.import(&prebuilt).unwrap(),
        MiniAppShareImportEntry::PrebuiltRelease(_)
    ));

    fs::write(prebuilt.join("files/ui/index.html"), b"tampered").unwrap();
    assert!(matches!(
        filesystem.import_prebuilt_release(&prebuilt),
        Err(MiniAppShareBundleError::Tampered(_))
    ));
}

#[cfg(windows)]
#[test]
fn share_bundle_rejects_junctions_without_following_them() {
    let fixture = Fixture::new();
    let filesystem = MiniAppShareBundleFilesystem::default();
    let bundle = fixture.temp.path().join("junction-bundle");
    filesystem
        .export(fixture.export_request(true), &bundle)
        .unwrap();

    let outside = fixture.temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let marker = outside.join("keep.txt");
    fs::write(&marker, b"keep").unwrap();
    let injected = bundle.join("source/files/injected");
    junction::create(&outside, &injected).unwrap();

    assert!(matches!(
        filesystem.import_share_bundle(&bundle),
        Err(MiniAppShareBundleError::InvalidInventory(_))
    ));
    assert_eq!(fs::read(&marker).unwrap(), b"keep");
    junction::delete(&injected).unwrap();
}

struct Fixture {
    temp: tempfile::TempDir,
    source: nomifun_miniapp_platform::MiniAppSourceSnapshot,
    release: nomifun_miniapp_platform::MiniAppStoredRelease,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source_store = MiniAppSourceStore::new(temp.path().join("source-store")).unwrap();
        let project = source_store
            .create_project("owner-1", "miniapp-1", "project-1", "Notes")
            .unwrap();
        let replaced = source_store
            .replace_source(
                "owner-1",
                "miniapp-1",
                "project-1",
                &project.source_snapshot_digest,
                vec![
                    MiniAppSourceFileInput::new(
                        "ui/index.html",
                        b"<main>Notes</main>".to_vec(),
                    ),
                    MiniAppSourceFileInput::new(
                        "ui/app.js",
                        b"console.log('notes');".to_vec(),
                    ),
                ],
            )
            .unwrap();
        let source = source_store
            .read_snapshot(
                "owner-1",
                "miniapp-1",
                "project-1",
                &replaced.source_snapshot_digest,
            )
            .unwrap();
        let artifact = MiniAppStaticBundleBuilder::new()
            .build_ui_only(static_input(&source))
            .unwrap();
        let release_files = artifact
            .files
            .iter()
            .map(|file| {
                let source_bytes = source.file(&file.normalized_relative_path).unwrap();
                let bytes = if file.normalized_relative_path == "ui/index.html" {
                    materialize_surface_entrypoint(source_bytes).unwrap()
                } else {
                    source_bytes.to_vec()
                };
                MiniAppReleaseFileBytes::new(file.normalized_relative_path.clone(), bytes)
            })
            .collect::<Vec<_>>();
        let release_store =
            MiniAppReleaseStore::new(temp.path().join("release-store")).unwrap();
        let release = release_store
            .publish(MiniAppReleasePublishRequest::ui_only(
                MiniAppSourceScope::new("owner-1", "miniapp-1", "project-1").unwrap(),
                source.source_snapshot_digest.clone(),
                source.dependency_lock_digest.clone(),
                source.project.build_generation,
                artifact,
                release_files,
            ))
            .unwrap()
            .stored;
        Self {
            temp,
            source,
            release,
        }
    }

    fn export_request(&self, include_source: bool) -> MiniAppShareBundleExport<'_> {
        MiniAppShareBundleExport {
            bundle_id: MiniAppShareBundleId::from(format!(
                "bundle-{}",
                if include_source { "source" } else { "runtime" }
            )),
            source_miniapp_id: Some(MiniAppId::from("miniapp-1")),
            release: &self.release,
            source: include_source.then(|| MiniAppShareSourceExport {
                snapshot: &self.source,
                source_archive_artifact_id: ArtifactId::from("source-artifact-1"),
                dependency_lock_artifact_id: ArtifactId::from("lock-artifact-1"),
            }),
            test_provenance: None,
        }
    }
}

fn static_input(
    source: &nomifun_miniapp_platform::MiniAppSourceSnapshot,
) -> MiniAppStaticBundleInput {
    MiniAppStaticBundleInput {
        artifact_id: ArtifactId::from("release-artifact-1"),
        display: LocalizedMetadata {
            name: "Notes".into(),
            description: "Share test".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui_index_html: source.file("ui/index.html").unwrap().to_vec(),
        ui_assets: vec![MiniAppStaticBundleFile::new(
            "ui/app.js",
            source.file("ui/app.js").unwrap().to_vec(),
        )],
        service: None,
        package_json: Some(br#"{"scripts":{}}"#.to_vec()),
        dependency_lock_digest: source.dependency_lock_digest.clone(),
        dependency_graph_digest: digest_bytes(b"graph"),
        config_schema: StrictJsonValue(serde_json::json!({"type": "object"})),
        credential_slots: Vec::new(),
        resource_contract: MiniAppResourceContract::default(),
        schemas: BTreeMap::new(),
        bridge_contract_digest: digest_bytes(b"bridge"),
        contribution_package: PackageRef {
            id: "miniapp.share-test".into(),
            version: "1.0.0".into(),
        },
        contributions: Default::default(),
        migrations: Vec::new(),
    }
}

fn copy_tree(source: &std::path::Path, destination: &std::path::Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
