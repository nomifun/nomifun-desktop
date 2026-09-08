use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, ArtifactEnvelope, ArtifactId, DigestHex,
    LocalizedMetadata, MiniAppResourceContract, StrictJsonValue, MINIAPP_RELEASE_PROFILE_VERSION,
};
use nomifun_miniapp_platform::{
    MiniAppReleaseArtifactIdentity, MiniAppReleaseFileBytes, MiniAppReleasePublishRequest,
    MiniAppReleaseStore, MiniAppSourceFileInput, MiniAppSourceScope, MiniAppSourceStore,
    MiniAppSourceStoreError, MiniAppStaticBundleBuilder, MiniAppStaticBundleInput,
    materialize_surface_entrypoint,
};
use serde_json::{Value, json};
use uuid::Uuid;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("nomifun-miniapp-{label}-{}", Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scope() -> MiniAppSourceScope {
    MiniAppSourceScope::new("owner-1", "miniapp-1", "project-1").unwrap()
}

fn digest(bytes: &[u8]) -> DigestHex {
    digest_bytes(bytes)
}

#[test]
fn source_create_returns_editable_default_and_exact_snapshot() {
    let root = TestRoot::new("source-create");
    let store = MiniAppSourceStore::new(root.path()).unwrap();
    let project = store
        .create_project("owner-1", "miniapp-1", "project-1", "Notes")
        .unwrap();

    assert_eq!(project.source_revision, 1);
    assert_eq!(project.build_generation, 1);
    assert_eq!(
        project.build_profile_version.as_ref(),
        MINIAPP_RELEASE_PROFILE_VERSION
    );
    assert!(project
        .managed_relative_path
        .ends_with("sources/owner-1/miniapps/miniapp-1/projects/project-1/source"));

    let snapshot = store
        .read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
        )
        .unwrap();
    assert_eq!(snapshot.project, project);
    assert!(snapshot.file("ui/index.html").is_some());
    assert_eq!(
        snapshot.dependency_lock_digest,
        project.dependency_lock_digest
    );
    assert_eq!(
        digest(&snapshot.dependency_lock),
        project.dependency_lock_digest
    );

    assert!(matches!(
        store.read_snapshot(
            "owner-2",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest
        ),
        Err(MiniAppSourceStoreError::ProjectNotFound)
    ));
}

#[test]
fn source_replace_is_owner_cas_and_rejects_service_or_unsafe_paths() {
    let root = TestRoot::new("source-replace");
    let store = MiniAppSourceStore::new(root.path()).unwrap();
    let project = store
        .create_project("owner-1", "miniapp-1", "project-1", "Notes")
        .unwrap();

    let next = store
        .replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
            vec![
                MiniAppSourceFileInput::new("ui/index.html", b"<html>updated</html>".to_vec()),
                MiniAppSourceFileInput::new("ui/app.js", b"export const ok = true;".to_vec()),
            ],
        )
        .unwrap();
    assert_eq!(next.source_revision, 2);
    assert_eq!(next.build_generation, 2);

    assert!(matches!(
        store.replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
            vec![MiniAppSourceFileInput::new(
                "ui/index.html",
                b"stale".to_vec()
            )]
        ),
        Err(MiniAppSourceStoreError::CompareAndSwapConflict { .. })
    ));
    assert!(store
        .replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &next.source_snapshot_digest,
            vec![
                MiniAppSourceFileInput::new("ui/index.html", b"ok".to_vec()),
                MiniAppSourceFileInput::new("service/main.mjs", b"forbidden".to_vec()),
            ],
        )
        .is_err());
    assert!(store
        .replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &next.source_snapshot_digest,
            vec![MiniAppSourceFileInput::new("../escape", b"no".to_vec())],
        )
        .is_err());
}

#[test]
fn release_publish_loads_immutable_bytes_and_keeps_owner_project_boundaries() {
    let source_root = TestRoot::new("release-source");
    let source_store = MiniAppSourceStore::new(source_root.path()).unwrap();
    let source = source_store
        .create_project("owner-1", "miniapp-1", "project-1", "Notes")
        .unwrap();
    let source_snapshot = source_store
        .read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &source.source_snapshot_digest,
        )
        .unwrap();

    let artifact = MiniAppStaticBundleBuilder::new()
        .build_ui_only(static_input(
            source_snapshot
                .files
                .iter()
                .map(|file| {
                    (
                        file.normalized_relative_path.clone(),
                        file.bytes.clone(),
                    )
                })
                .collect(),
            source_snapshot.dependency_lock_digest.clone(),
        ))
        .unwrap();
    let file_bytes = artifact
        .files
        .iter()
        .map(|file| MiniAppReleaseFileBytes::new(
            file.normalized_relative_path.clone(),
            if file.normalized_relative_path == "ui/index.html" {
                materialize_surface_entrypoint(
                    source_snapshot.file(&file.normalized_relative_path).unwrap(),
                )
                .unwrap()
            } else {
                source_snapshot
                    .file(&file.normalized_relative_path)
                    .unwrap()
                    .to_vec()
            },
        ))
        .collect::<Vec<_>>();
    let request = MiniAppReleasePublishRequest::ui_only(
        scope(),
        source.source_snapshot_digest.clone(),
        source.dependency_lock_digest.clone(),
        source.build_generation,
        artifact.clone(),
        file_bytes.clone(),
    );

    let release_root = TestRoot::new("release-store");
    let store = MiniAppReleaseStore::new(release_root.path()).unwrap();
    let first = store.publish(request.clone()).unwrap();
    assert!(!first.already_present);
    assert_eq!(first.stored.artifact, artifact);
    assert_eq!(first.stored.files, file_bytes);
    assert!(first.stored.managed_relative_path.contains("owner-1"));
    assert_eq!(
        first.stored.manifest_bytes,
        canonical_json_bytes(&first.stored.artifact.manifest).unwrap()
    );

    let second = store.publish(request).unwrap();
    assert!(second.already_present);
    let mut rebuilt_artifact = artifact.clone();
    rebuilt_artifact.artifact_id = ArtifactId::from("artifact-2");
    let rebuilt = store
        .publish(MiniAppReleasePublishRequest::ui_only(
            scope(),
            digest(b"later-source-snapshot"),
            source.dependency_lock_digest.clone(),
            source.build_generation + 1,
            rebuilt_artifact,
            file_bytes.clone(),
        ))
        .unwrap();
    assert!(rebuilt.already_present);
    assert_eq!(
        rebuilt.stored.artifact.artifact_id,
        artifact.artifact_id,
        "content-addressed Artifact reuse keeps the first immutable identity"
    );
    let loaded = store
        .load(scope(), artifact.artifact_digest.as_ref())
        .unwrap();
    assert_eq!(loaded.files, file_bytes);
    let expected_identity = MiniAppReleaseArtifactIdentity::from_artifact(&artifact);
    assert_eq!(
        store
            .load_exact(scope(), &expected_identity)
            .unwrap()
            .artifact,
        artifact
    );
    let mut wrong_identity = expected_identity.clone();
    wrong_identity.artifact_id = ArtifactId::from("artifact-wrong");
    assert!(
        store.load_exact(scope(), &wrong_identity).is_err(),
        "an exact Artifact load must reject an independently wrong artifact_id"
    );

    let artifact_record_path = loaded.artifact_root.join("artifact.json");
    let canonical_artifact_record = fs::read(&artifact_record_path).unwrap();
    let artifact_record: serde_json::Value = serde_json::from_slice(
        &canonical_artifact_record,
    )
    .unwrap();
    assert_eq!(artifact_record["format_version"], "2.0.0");
    assert!(artifact_record["artifact"]["payload"]["artifact_id"].is_string());
    for release_lineage_field in [
        "source_snapshot_digest",
        "dependency_lock_digest",
        "build_generation",
    ] {
        assert!(
            artifact_record.get(release_lineage_field).is_none(),
            "content-addressed Artifact storage must not impersonate Release lineage"
        );
    }

    let mut tampered_artifact_id = artifact_record.clone();
    tampered_artifact_id["artifact"]["payload"]["artifact_id"] =
        Value::String("artifact-tampered".into());
    fs::write(
        &artifact_record_path,
        canonical_json_bytes(&tampered_artifact_id).unwrap(),
    )
    .unwrap();
    assert!(
        store.load(scope(), artifact.artifact_digest.as_ref()).is_err(),
        "changing only artifact_id must fail the Artifact identity envelope"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let mut replacement_artifact = artifact.clone();
    replacement_artifact.artifact_id = ArtifactId::from("artifact-replaced");
    let replacement_envelope = ArtifactEnvelope::new(replacement_artifact).unwrap();
    let mut valid_but_wrong_identity_record = artifact_record.clone();
    valid_but_wrong_identity_record["artifact"] =
        serde_json::to_value(replacement_envelope).unwrap();
    fs::write(
        &artifact_record_path,
        canonical_json_bytes(&valid_but_wrong_identity_record).unwrap(),
    )
    .unwrap();
    assert!(
        store.load_exact(scope(), &expected_identity).is_err(),
        "an exact caller identity must reject a valid content-equivalent Artifact with another artifact_id"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let mut tampered_artifact_field = artifact_record.clone();
    tampered_artifact_field["artifact"]["payload"]["manifest"]["payload_digest"] =
        Value::String(digest(b"wrong-manifest").0);
    fs::write(
        &artifact_record_path,
        canonical_json_bytes(&tampered_artifact_field).unwrap(),
    )
    .unwrap();
    assert!(
        store.load(scope(), artifact.artifact_digest.as_ref()).is_err(),
        "a typed Artifact field mismatch must fail before bytes are returned"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let mut unknown_field = artifact_record.clone();
    unknown_field["unexpected"] = json!("must fail closed");
    fs::write(
        &artifact_record_path,
        canonical_json_bytes(&unknown_field).unwrap(),
    )
    .unwrap();
    assert!(
        store.load(scope(), artifact.artifact_digest.as_ref()).is_err(),
        "unknown Artifact record fields must be rejected"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let mut object_only_record = artifact_record.clone();
    object_only_record["artifact"] = json!({
        "artifact_id": artifact.artifact_id.clone(),
        "artifact_digest": artifact.artifact_digest.clone(),
        "manifest": artifact.manifest.clone(),
        "files": artifact.files.clone(),
    });
    fs::write(
        &artifact_record_path,
        canonical_json_bytes(&object_only_record).unwrap(),
    )
    .unwrap();
    assert!(
        store.load(scope(), artifact.artifact_digest.as_ref()).is_err(),
        "an arbitrary JSON object must not substitute for the typed identity envelope"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let mut non_canonical_record = canonical_artifact_record.clone();
    non_canonical_record.extend_from_slice(b"\n");
    fs::write(&artifact_record_path, &non_canonical_record).unwrap();
    assert!(
        store.load(scope(), artifact.artifact_digest.as_ref()).is_err(),
        "Artifact records must be byte-for-byte canonical JSON"
    );
    fs::write(&artifact_record_path, &canonical_artifact_record).unwrap();

    let tampered = loaded
        .artifact_root
        .join("files")
        .join("ui")
        .join("index.html");
    fs::write(&tampered, b"tampered").unwrap();
    assert!(store
        .load(scope(), artifact.artifact_digest.as_ref())
        .is_err());

    let other_scope =
        MiniAppSourceScope::new("owner-2", "miniapp-1", "project-1").unwrap();
    assert!(store
        .load(other_scope, artifact.artifact_digest.as_ref())
        .is_err());
}

fn static_input(
    bytes: BTreeMap<String, Vec<u8>>,
    dependency_lock_digest: DigestHex,
) -> MiniAppStaticBundleInput {
    let index = bytes.get("ui/index.html").unwrap().clone();
    let ui_assets = bytes
        .into_iter()
        .filter(|(path, _)| path != "ui/index.html")
        .map(|(path, bytes)| {
            nomifun_miniapp_platform::MiniAppStaticBundleFile::new(path, bytes)
        })
        .collect();
    MiniAppStaticBundleInput {
        artifact_id: ArtifactId::from("artifact-1"),
        display: LocalizedMetadata {
            name: "Notes".into(),
            description: "test".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui_index_html: index,
        ui_assets,
        service: None,
        package_json: Some(br#"{"scripts":{}}"#.to_vec()),
        dependency_lock_digest,
        dependency_graph_digest: digest(b"graph"),
        config_schema: StrictJsonValue(serde_json::json!({"type": "object"})),
        credential_slots: Vec::new(),
        resource_contract: MiniAppResourceContract::default(),
        schemas: BTreeMap::new(),
        bridge_contract_digest: digest(b"bridge"),
        contribution_package: nomifun_agent_contracts::PackageRef {
            id: "miniapp.test".into(),
            version: "1.0.0".into(),
        },
        contributions: Default::default(),
        migrations: Vec::new(),
    }
}
