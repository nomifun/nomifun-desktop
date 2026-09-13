use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, ArtifactEnvelope, ArtifactId, DigestHex,
    LocalizedMetadata, MiniAppResourceContract, MiniAppServiceLifecycle, StrictJsonValue,
    VersionString, MINIAPP_RELEASE_PROFILE_VERSION,
};
use nomifun_plugin_platform::runtime::{
    materialize_surface_entrypoint, PluginRuntimeDependencyLockV1, PluginRuntimeReleaseArtifactIdentity,
    PluginRuntimeReleaseFileBytes, PluginRuntimeReleasePublishRequest, PluginRuntimeReleaseStore,
    PluginRuntimeSourceContentKind, PluginRuntimeSourceExactImportRequest, PluginRuntimeSourceFileInput,
    PluginRuntimeSourceScope, PluginRuntimeSourceSnapshot, PluginRuntimeSourceStore, PluginRuntimeSourceStoreError,
    PluginRuntimeStaticBundleBuilder, PluginRuntimeStaticBundleInput, PluginRuntimeStaticServiceInput,
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

fn scope() -> PluginRuntimeSourceScope {
    PluginRuntimeSourceScope::new("owner-1", "miniapp-1", "project-1").unwrap()
}

fn digest(bytes: &[u8]) -> DigestHex {
    digest_bytes(bytes)
}

fn exact_import_request(
    snapshot: &PluginRuntimeSourceSnapshot,
    scope: PluginRuntimeSourceScope,
    display_name: &str,
    content_kind: PluginRuntimeSourceContentKind,
) -> PluginRuntimeSourceExactImportRequest {
    PluginRuntimeSourceExactImportRequest {
        scope,
        display_name: display_name.into(),
        content_kind,
        dependency_lock: snapshot.dependency_lock.clone(),
        files: snapshot
            .files
            .iter()
            .map(|file| {
                PluginRuntimeSourceFileInput::new(
                    file.normalized_relative_path.clone(),
                    file.bytes.clone(),
                )
            })
            .collect(),
        expected_source_snapshot_digest: snapshot.source_snapshot_digest.clone(),
        expected_dependency_lock_digest: snapshot.dependency_lock_digest.clone(),
        build_generation: snapshot.project.build_generation,
        build_profile_version: snapshot.project.build_profile_version.clone(),
    }
}

#[test]
fn source_create_returns_editable_default_and_exact_snapshot() {
    let root = TestRoot::new("source-create");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
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
        Err(PluginRuntimeSourceStoreError::ProjectNotFound)
    ));
}

#[test]
fn source_replace_is_owner_cas_and_rejects_service_or_unsafe_paths() {
    let root = TestRoot::new("source-replace");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
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
                PluginRuntimeSourceFileInput::new("ui/index.html", b"<html>updated</html>".to_vec()),
                PluginRuntimeSourceFileInput::new("ui/app.js", b"export const ok = true;".to_vec()),
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
            vec![PluginRuntimeSourceFileInput::new(
                "ui/index.html",
                b"stale".to_vec()
            )]
        ),
        Err(PluginRuntimeSourceStoreError::CompareAndSwapConflict { .. })
    ));
    assert!(store
        .replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &next.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new("ui/index.html", b"ok".to_vec()),
                PluginRuntimeSourceFileInput::new("service/main.mjs", b"forbidden".to_vec()),
            ],
        )
        .is_err());
    assert!(store
        .replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &next.source_snapshot_digest,
            vec![PluginRuntimeSourceFileInput::new("../escape", b"no".to_vec())],
        )
        .is_err());
}

#[test]
fn prepared_file_replace_is_atomic_preserves_other_files_and_rejects_a_stale_head() {
    let root = TestRoot::new("prepared-source-replace");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
    let project = store
        .create_project("owner-1", "miniapp-1", "project-1", "Notes")
        .unwrap();
    let before = store
        .current_snapshot("owner-1", "miniapp-1", "project-1")
        .unwrap();
    let original_app = before.file("ui/app.js").map(ToOwned::to_owned);

    let prepared = store
        .prepare_file_replace(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
            "ui/index.html",
            b"<html>prepared update</html>".to_vec(),
        )
        .unwrap();
    assert_eq!(
        store
            .current_snapshot("owner-1", "miniapp-1", "project-1")
            .unwrap()
            .source_snapshot_digest,
        project.source_snapshot_digest,
        "prepare must not publish a new Source head"
    );
    assert_eq!(prepared.expected_build_generation, 1);
    assert_eq!(prepared.next_build_generation, 2);

    let committed = store.commit_prepared_source(&prepared).unwrap();
    assert_eq!(committed.source_snapshot_digest, prepared.next_source_snapshot_digest);
    assert_eq!(committed.build_generation, 2);
    let after = store
        .current_snapshot("owner-1", "miniapp-1", "project-1")
        .unwrap();
    assert_eq!(after.file("ui/index.html"), Some(b"<html>prepared update</html>".as_slice()));
    assert_eq!(after.file("ui/app.js").map(ToOwned::to_owned), original_app);
    assert!(matches!(
        store.commit_prepared_source(&prepared),
        Err(PluginRuntimeSourceStoreError::CompareAndSwapConflict { .. })
    ));

    let noop = store
        .prepare_file_replace(
            "owner-1",
            "miniapp-1",
            "project-1",
            &after.source_snapshot_digest,
            "ui/index.html",
            after.file("ui/index.html").unwrap().to_vec(),
        )
        .unwrap();
    assert!(noop.is_noop());
    assert_eq!(noop.expected_build_generation, noop.next_build_generation);
}

#[test]
fn service_source_round_trips_exact_main_module_without_relaxing_ui_only_paths() {
    let root = TestRoot::new("service-source");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
    let service_main = b"export async function invoke(method, payload) { return { method, payload }; }\n";
    assert!(matches!(
        store.create_service_project(
            "owner-1",
            "miniapp-1",
            "project-failed",
            "Broken Service",
            Vec::new(),
        ),
        Err(PluginRuntimeSourceStoreError::EmptyFile { path })
            if path == "service/main.mjs"
    ));
    assert_eq!(
        fs::read_dir(store.staging_root()).unwrap().count(),
        0,
        "failed Service Source creation must clean its private staging tree"
    );

    let project = store
        .create_service_project(
            "owner-1",
            "miniapp-1",
            "project-1",
            "Service Notes",
            service_main.to_vec(),
        )
        .unwrap();
    let snapshot = store
        .read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
        )
        .unwrap();

    assert_eq!(snapshot.content_kind(), PluginRuntimeSourceContentKind::Service);
    assert_eq!(snapshot.service_main_mjs(), Some(service_main.as_slice()));
    assert_eq!(
        snapshot
            .files
            .iter()
            .find(|file| file.normalized_relative_path == "service/main.mjs")
            .unwrap()
            .digest,
        digest(service_main)
    );

    let replaced = store
        .replace_service_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &project.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new(
                    "ui/index.html",
                    b"<!doctype html><title>service</title>".to_vec(),
                ),
                PluginRuntimeSourceFileInput::new(
                    "service/main.mjs",
                    b"export async function invoke() { return 'updated'; }\n".to_vec(),
                ),
            ],
        )
        .unwrap();
    assert_eq!(replaced.source_revision, 2);
    assert!(store
        .replace_service_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &replaced.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new("ui/index.html", b"ok".to_vec()),
                PluginRuntimeSourceFileInput::new(
                    "service/helper.mjs",
                    b"export const forbidden = true;\n".to_vec(),
                ),
            ],
        )
        .is_err());
    assert!(store
        .replace_service_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &replaced.source_snapshot_digest,
            vec![PluginRuntimeSourceFileInput::new(
                "ui/index.html",
                b"missing service".to_vec(),
            )],
        )
        .is_err());
    assert!(matches!(
        store.replace_source(
            "owner-1",
            "miniapp-1",
            "project-1",
            &replaced.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new("ui/index.html", b"ok".to_vec()),
                PluginRuntimeSourceFileInput::new(
                    "service/main.mjs",
                    b"export default {};\n".to_vec(),
                ),
            ],
        ),
        Err(PluginRuntimeSourceStoreError::ServiceSourceForbidden { .. })
    ));
}

#[test]
fn source_exact_import_round_trips_ui_and_service_without_default_source() {
    let root = TestRoot::new("source-exact-import-roundtrip");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();

    let ui = store
        .create_project("owner-origin", "miniapp-ui", "project-ui", "Origin UI")
        .unwrap();
    let ui = store
        .replace_source(
            "owner-origin",
            "miniapp-ui",
            "project-ui",
            &ui.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new(
                    "ui/index.html",
                    b"<!doctype html><main id=\"imported-ui\"></main>".to_vec(),
                ),
                PluginRuntimeSourceFileInput::new(
                    "ui/app.js",
                    b"globalThis.__exactImport = 'ui';\n".to_vec(),
                ),
            ],
        )
        .unwrap();
    let ui_snapshot = store
        .read_snapshot(
            "owner-origin",
            "miniapp-ui",
            "project-ui",
            &ui.source_snapshot_digest,
        )
        .unwrap();
    let dependency_lock = PluginRuntimeDependencyLockV1 {
        format_version: VersionString::from("1.0.0"),
        dependencies: BTreeMap::from([
            ("nomifun-runtime".into(), "1.2.3".into()),
            ("zod".into(), "4.0.0".into()),
        ]),
    };
    let dependency_lock_bytes = dependency_lock.canonical_bytes().unwrap();
    let dependency_lock_digest = dependency_lock.digest().unwrap();
    let mut ui_import = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new("owner-import", "miniapp-ui-copy", "project-ui-copy").unwrap(),
        "Imported UI",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    ui_import.dependency_lock = dependency_lock_bytes.clone();
    ui_import.expected_dependency_lock_digest = dependency_lock_digest.clone();
    ui_import.build_generation = 42;

    let imported_ui = store.import_project_exact(ui_import).unwrap();
    assert_eq!(imported_ui.display_name, "Imported UI");
    assert_eq!(imported_ui.source_revision, 1);
    assert_eq!(imported_ui.build_generation, 42);
    assert_eq!(
        imported_ui.source_snapshot_digest,
        ui_snapshot.source_snapshot_digest
    );
    assert_eq!(
        imported_ui.dependency_lock_digest,
        dependency_lock_digest
    );
    let imported_ui_snapshot = store
        .read_snapshot(
            "owner-import",
            "miniapp-ui-copy",
            "project-ui-copy",
            &imported_ui.source_snapshot_digest,
        )
        .unwrap();
    assert_eq!(imported_ui_snapshot.files, ui_snapshot.files);
    assert_eq!(imported_ui_snapshot.dependency_lock, dependency_lock_bytes);
    assert_eq!(
        imported_ui_snapshot.content_kind(),
        PluginRuntimeSourceContentKind::UiOnly
    );
    assert_eq!(
        imported_ui_snapshot.file("ui/index.html"),
        Some(b"<!doctype html><main id=\"imported-ui\"></main>".as_slice())
    );

    let service = store
        .create_service_project(
            "owner-origin",
            "miniapp-service",
            "project-service",
            "Origin Service",
            b"export async function invoke() { return 'origin'; }\n".to_vec(),
        )
        .unwrap();
    let service = store
        .replace_service_source(
            "owner-origin",
            "miniapp-service",
            "project-service",
            &service.source_snapshot_digest,
            vec![
                PluginRuntimeSourceFileInput::new(
                    "ui/index.html",
                    b"<!doctype html><main id=\"service-ui\"></main>".to_vec(),
                ),
                PluginRuntimeSourceFileInput::new(
                    "ui/client.js",
                    b"globalThis.__exactImport = 'service';\n".to_vec(),
                ),
                PluginRuntimeSourceFileInput::new(
                    "service/main.mjs",
                    b"export async function invoke(method) { return `copied:${method}`; }\n"
                        .to_vec(),
                ),
            ],
        )
        .unwrap();
    let service_snapshot = store
        .read_snapshot(
            "owner-origin",
            "miniapp-service",
            "project-service",
            &service.source_snapshot_digest,
        )
        .unwrap();
    let service_import = exact_import_request(
        &service_snapshot,
        PluginRuntimeSourceScope::new(
            "owner-import",
            "miniapp-service-copy",
            "project-service-copy",
        )
        .unwrap(),
        "Imported Service",
        PluginRuntimeSourceContentKind::Service,
    );

    let imported_service = store.import_project_exact(service_import).unwrap();
    let imported_service_snapshot = store
        .read_snapshot(
            "owner-import",
            "miniapp-service-copy",
            "project-service-copy",
            &imported_service.source_snapshot_digest,
        )
        .unwrap();
    assert_eq!(imported_service.source_revision, 1);
    assert_eq!(
        imported_service.build_generation,
        service_snapshot.project.build_generation
    );
    assert_eq!(imported_service_snapshot.files, service_snapshot.files);
    assert_eq!(
        imported_service_snapshot.content_kind(),
        PluginRuntimeSourceContentKind::Service
    );
    assert_eq!(
        imported_service_snapshot.service_main_mjs(),
        service_snapshot.service_main_mjs()
    );
}

#[test]
fn source_exact_import_rejects_tampering_and_kind_or_lineage_drift() {
    let root = TestRoot::new("source-exact-import-reject");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
    let ui = store
        .create_project("owner-origin", "miniapp-ui", "project-ui", "Origin UI")
        .unwrap();
    let ui_snapshot = store
        .read_snapshot(
            "owner-origin",
            "miniapp-ui",
            "project-ui",
            &ui.source_snapshot_digest,
        )
        .unwrap();

    let mut source_tamper = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new("owner-import", "miniapp-tamper", "project-tamper").unwrap(),
        "Tampered",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    source_tamper.files[0].bytes.extend_from_slice(b"tampered");
    assert!(matches!(
        store.import_project_exact(source_tamper),
        Err(PluginRuntimeSourceStoreError::DigestMismatch { .. })
    ));

    let changed_lock = PluginRuntimeDependencyLockV1 {
        format_version: VersionString::from("1.0.0"),
        dependencies: BTreeMap::from([("changed".into(), "1.0.0".into())]),
    }
    .canonical_bytes()
    .unwrap();
    let mut lock_tamper = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new("owner-import", "miniapp-lock", "project-lock").unwrap(),
        "Lock Tamper",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    lock_tamper.dependency_lock = changed_lock;
    assert!(matches!(
        store.import_project_exact(lock_tamper),
        Err(PluginRuntimeSourceStoreError::DigestMismatch { .. })
    ));

    let ui_as_service = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new("owner-import", "miniapp-kind-ui", "project-kind-ui").unwrap(),
        "Wrong Kind",
        PluginRuntimeSourceContentKind::Service,
    );
    assert!(matches!(
        store.import_project_exact(ui_as_service),
        Err(PluginRuntimeSourceStoreError::InvalidRecord(_))
    ));

    let service = store
        .create_service_project(
            "owner-origin",
            "miniapp-service",
            "project-service",
            "Origin Service",
            b"export async function invoke() { return true; }\n".to_vec(),
        )
        .unwrap();
    let service_snapshot = store
        .read_snapshot(
            "owner-origin",
            "miniapp-service",
            "project-service",
            &service.source_snapshot_digest,
        )
        .unwrap();
    let service_as_ui = exact_import_request(
        &service_snapshot,
        PluginRuntimeSourceScope::new(
            "owner-import",
            "miniapp-kind-service",
            "project-kind-service",
        )
        .unwrap(),
        "Wrong Kind",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    assert!(matches!(
        store.import_project_exact(service_as_ui),
        Err(PluginRuntimeSourceStoreError::ServiceSourceForbidden { .. })
    ));

    let mut wrong_profile = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new(
            "owner-import",
            "miniapp-wrong-profile",
            "project-wrong-profile",
        )
        .unwrap(),
        "Wrong Profile",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    wrong_profile.build_profile_version = VersionString::from("0.0.0");
    assert!(matches!(
        store.import_project_exact(wrong_profile),
        Err(PluginRuntimeSourceStoreError::InvalidRecord(_))
    ));

    let mut zero_generation = exact_import_request(
        &ui_snapshot,
        PluginRuntimeSourceScope::new(
            "owner-import",
            "miniapp-zero-generation",
            "project-zero-generation",
        )
        .unwrap(),
        "Zero Generation",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    zero_generation.build_generation = 0;
    assert!(matches!(
        store.import_project_exact(zero_generation),
        Err(PluginRuntimeSourceStoreError::InvalidRecord(_))
    ));
    assert_eq!(
        fs::read_dir(store.staging_root()).unwrap().count(),
        0,
        "all rejected exact imports must clean their private staging trees"
    );
}

#[test]
fn source_exact_import_rejects_duplicate_target_and_preserves_owner_scope() {
    let root = TestRoot::new("source-exact-import-owner");
    let store = PluginRuntimeSourceStore::new(root.path()).unwrap();
    let origin = store
        .create_project(
            "owner-origin",
            "miniapp-origin",
            "project-origin",
            "Origin",
        )
        .unwrap();
    let snapshot = store
        .read_snapshot(
            "owner-origin",
            "miniapp-origin",
            "project-origin",
            &origin.source_snapshot_digest,
        )
        .unwrap();
    let request = exact_import_request(
        &snapshot,
        PluginRuntimeSourceScope::new("owner-1", "miniapp-copy", "project-copy").unwrap(),
        "Owner One",
        PluginRuntimeSourceContentKind::UiOnly,
    );

    store.import_project_exact(request.clone()).unwrap();
    assert!(matches!(
        store.import_project_exact(request),
        Err(PluginRuntimeSourceStoreError::ProjectAlreadyExists)
    ));

    let mut other_owner = exact_import_request(
        &snapshot,
        PluginRuntimeSourceScope::new("owner-2", "miniapp-copy", "project-copy").unwrap(),
        "Owner Two",
        PluginRuntimeSourceContentKind::UiOnly,
    );
    other_owner.build_generation = 7;
    let owner_two = store.import_project_exact(other_owner).unwrap();
    assert_eq!(owner_two.owner_id, "owner-2");
    assert_eq!(owner_two.build_generation, 7);
    assert!(store
        .read_snapshot(
            "owner-1",
            "miniapp-copy",
            "project-copy",
            &snapshot.source_snapshot_digest,
        )
        .is_ok());
    assert!(store
        .read_snapshot(
            "owner-2",
            "miniapp-copy",
            "project-copy",
            &snapshot.source_snapshot_digest,
        )
        .is_ok());
    assert!(matches!(
        store.read_snapshot(
            "owner-3",
            "miniapp-copy",
            "project-copy",
            &snapshot.source_snapshot_digest,
        ),
        Err(PluginRuntimeSourceStoreError::ProjectNotFound)
    ));
    assert_eq!(fs::read_dir(store.staging_root()).unwrap().count(), 0);
}

#[test]
fn release_publish_loads_immutable_bytes_and_keeps_owner_project_boundaries() {
    let source_root = TestRoot::new("release-source");
    let source_store = PluginRuntimeSourceStore::new(source_root.path()).unwrap();
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

    let artifact = PluginRuntimeStaticBundleBuilder::new()
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
        .map(|file| PluginRuntimeReleaseFileBytes::new(
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
    let request = PluginRuntimeReleasePublishRequest::ui_only(
        scope(),
        source.source_snapshot_digest.clone(),
        source.dependency_lock_digest.clone(),
        source.build_generation,
        artifact.clone(),
        file_bytes.clone(),
    );

    let release_root = TestRoot::new("release-store");
    let store = PluginRuntimeReleaseStore::new(release_root.path()).unwrap();
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
        .publish(PluginRuntimeReleasePublishRequest::ui_only(
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
    let expected_identity = PluginRuntimeReleaseArtifactIdentity::from_artifact(&artifact);
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
        PluginRuntimeSourceScope::new("owner-2", "miniapp-1", "project-1").unwrap();
    assert!(store
        .load(other_scope, artifact.artifact_digest.as_ref())
        .is_err());
}

#[test]
fn service_release_publishes_and_loads_exact_main_module() {
    let source_root = TestRoot::new("service-release-source");
    let source_store = PluginRuntimeSourceStore::new(source_root.path()).unwrap();
    let service_main =
        b"export async function invoke(method, payload) { return { method, payload }; }\n";
    let source = source_store
        .create_service_project(
            "owner-1",
            "miniapp-1",
            "project-1",
            "Service Notes",
            service_main.to_vec(),
        )
        .unwrap();
    let snapshot = source_store
        .read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &source.source_snapshot_digest,
        )
        .unwrap();
    let input_files = snapshot
        .files
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.clone(),
                file.bytes.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let artifact = PluginRuntimeStaticBundleBuilder::new()
        .build(service_static_input(
            input_files,
            snapshot.dependency_lock_digest.clone(),
        ))
        .unwrap();
    let file_bytes = artifact
        .files
        .iter()
        .map(|file| {
            let source_bytes = snapshot.file(&file.normalized_relative_path).unwrap();
            let bytes = if file.normalized_relative_path == "ui/index.html" {
                materialize_surface_entrypoint(source_bytes).unwrap()
            } else {
                source_bytes.to_vec()
            };
            PluginRuntimeReleaseFileBytes::new(file.normalized_relative_path.clone(), bytes)
        })
        .collect::<Vec<_>>();

    let release_root = TestRoot::new("service-release-store");
    let store = PluginRuntimeReleaseStore::new(release_root.path()).unwrap();
    assert!(store
        .publish(PluginRuntimeReleasePublishRequest::ui_only(
            scope(),
            source.source_snapshot_digest.clone(),
            source.dependency_lock_digest.clone(),
            source.build_generation,
            artifact.clone(),
            file_bytes.clone(),
        ))
        .is_err());

    let mut mismatched_service_bytes = file_bytes.clone();
    mismatched_service_bytes
        .iter_mut()
        .find(|file| file.normalized_relative_path == "service/main.mjs")
        .unwrap()
        .bytes = b"export default async function changed() {};\n".to_vec();
    assert!(store
        .publish(PluginRuntimeReleasePublishRequest::service(
            scope(),
            source.source_snapshot_digest.clone(),
            source.dependency_lock_digest.clone(),
            source.build_generation,
            artifact.clone(),
            mismatched_service_bytes,
        ))
        .is_err());

    let mut forbidden_service_path = file_bytes.clone();
    forbidden_service_path.push(PluginRuntimeReleaseFileBytes::new(
        "service/helper.mjs",
        b"export const forbidden = true;\n".to_vec(),
    ));
    assert!(store
        .publish(PluginRuntimeReleasePublishRequest::service(
            scope(),
            source.source_snapshot_digest.clone(),
            source.dependency_lock_digest.clone(),
            source.build_generation,
            artifact.clone(),
            forbidden_service_path,
        ))
        .is_err());
    assert_eq!(
        fs::read_dir(store.staging_root()).unwrap().count(),
        0,
        "rejected Service Releases must not leave staging state"
    );

    let published = store
        .publish(PluginRuntimeReleasePublishRequest::service(
            scope(),
            source.source_snapshot_digest,
            source.dependency_lock_digest,
            source.build_generation,
            artifact.clone(),
            file_bytes.clone(),
        ))
        .unwrap();
    assert!(!published.already_present);
    assert_eq!(
        published
            .stored
            .files
            .iter()
            .find(|file| file.normalized_relative_path == "service/main.mjs")
            .unwrap()
            .bytes,
        service_main
    );
    assert_eq!(
        published
            .stored
            .artifact
            .manifest
            .payload
            .service
            .as_ref()
            .unwrap()
            .module_digest,
        digest(service_main)
    );
    assert_eq!(
        store
            .load(scope(), artifact.artifact_digest.as_ref())
            .unwrap()
            .files,
        file_bytes
    );
}

#[test]
fn source_and_release_project_purge_are_owner_scoped_and_idempotent() {
    let source_root = TestRoot::new("source-purge");
    let source_store = PluginRuntimeSourceStore::new(source_root.path()).unwrap();
    let first = source_store
        .create_project("owner-1", "miniapp-1", "project-1", "First")
        .unwrap();
    let second = source_store
        .create_project("owner-2", "miniapp-2", "project-2", "Second")
        .unwrap();

    let first_snapshot = source_store
        .read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &first.source_snapshot_digest,
        )
        .unwrap();
    let artifact = PluginRuntimeStaticBundleBuilder::new()
        .build_ui_only(static_input(
            first_snapshot
                .files
                .iter()
                .map(|file| {
                    (
                        file.normalized_relative_path.clone(),
                        file.bytes.clone(),
                    )
                })
                .collect(),
            first_snapshot.dependency_lock_digest.clone(),
        ))
        .unwrap();
    let file_bytes = artifact
        .files
        .iter()
        .map(|file| {
            let source_bytes = first_snapshot
                .file(&file.normalized_relative_path)
                .unwrap();
            PluginRuntimeReleaseFileBytes::new(
                file.normalized_relative_path.clone(),
                if file.normalized_relative_path == "ui/index.html" {
                    materialize_surface_entrypoint(source_bytes).unwrap()
                } else {
                    source_bytes.to_vec()
                },
            )
        })
        .collect::<Vec<_>>();
    let release_root = TestRoot::new("release-purge");
    let release_store = PluginRuntimeReleaseStore::new(release_root.path()).unwrap();
    release_store
        .publish(PluginRuntimeReleasePublishRequest::ui_only(
            scope(),
            first.source_snapshot_digest.clone(),
            first.dependency_lock_digest.clone(),
            first.build_generation,
            artifact,
            file_bytes,
        ))
        .unwrap();

    source_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .unwrap();
    release_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .unwrap();
    source_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .unwrap();
    release_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .unwrap();

    assert!(matches!(
        source_store.read_snapshot(
            "owner-1",
            "miniapp-1",
            "project-1",
            &first.source_snapshot_digest
        ),
        Err(PluginRuntimeSourceStoreError::ProjectNotFound)
    ));
    assert!(source_store
        .read_snapshot(
            "owner-2",
            "miniapp-2",
            "project-2",
            &second.source_snapshot_digest,
        )
        .is_ok());
    assert!(!release_root
        .path()
        .join("releases")
        .join("owner-1")
        .join("miniapps")
        .join("miniapp-1")
        .join("projects")
        .join("project-1")
        .exists());
}

#[cfg(windows)]
#[test]
fn project_purge_rejects_junction_parent_without_touching_external_data() {
    let source_root = TestRoot::new("source-purge-junction");
    let source_store = PluginRuntimeSourceStore::new(source_root.path()).unwrap();
    let source_owner = source_root.path().join("sources").join("owner-1");
    let outside_source_owner = source_root.path().join("outside-source-owner");
    let outside_source_project = outside_source_owner
        .join("miniapps")
        .join("miniapp-1")
        .join("projects")
        .join("project-1");
    fs::create_dir_all(&outside_source_project).unwrap();
    let source_marker = outside_source_project.join("keep.txt");
    fs::write(&source_marker, b"keep").unwrap();
    junction::create(&outside_source_owner, &source_owner).unwrap();

    assert!(source_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .is_err());
    assert_eq!(fs::read(&source_marker).unwrap(), b"keep");
    junction::delete(&source_owner).unwrap();

    let release_root = TestRoot::new("release-purge-junction");
    let release_store = PluginRuntimeReleaseStore::new(release_root.path()).unwrap();
    let release_owner = release_root.path().join("releases").join("owner-1");
    let outside_release_owner = release_root.path().join("outside-release-owner");
    let outside_release_project = outside_release_owner
        .join("miniapps")
        .join("miniapp-1")
        .join("projects")
        .join("project-1");
    fs::create_dir_all(outside_release_project.join("artifacts")).unwrap();
    let release_marker = outside_release_project.join("keep.txt");
    fs::write(&release_marker, b"keep").unwrap();
    junction::create(&outside_release_owner, &release_owner).unwrap();

    assert!(release_store
        .purge_project("owner-1", "miniapp-1", "project-1")
        .is_err());
    assert_eq!(fs::read(&release_marker).unwrap(), b"keep");
    junction::delete(&release_owner).unwrap();
}

fn static_input(
    bytes: BTreeMap<String, Vec<u8>>,
    dependency_lock_digest: DigestHex,
) -> PluginRuntimeStaticBundleInput {
    let index = bytes.get("ui/index.html").unwrap().clone();
    let ui_assets = bytes
        .into_iter()
        .filter(|(path, _)| path != "ui/index.html")
        .map(|(path, bytes)| {
            nomifun_plugin_platform::runtime::PluginRuntimeStaticBundleFile::new(path, bytes)
        })
        .collect();
    PluginRuntimeStaticBundleInput {
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

fn service_static_input(
    mut bytes: BTreeMap<String, Vec<u8>>,
    dependency_lock_digest: DigestHex,
) -> PluginRuntimeStaticBundleInput {
    let service_main = bytes.remove("service/main.mjs").unwrap();
    let mut input = static_input(bytes, dependency_lock_digest);
    input.service = Some(PluginRuntimeStaticServiceInput {
        main_mjs: service_main,
        lifecycle: MiniAppServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });
    input
}
