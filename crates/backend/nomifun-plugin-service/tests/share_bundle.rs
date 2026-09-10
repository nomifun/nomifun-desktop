use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, ArtifactFileDigest, ArtifactId, CapabilityActionDescriptor,
    CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind,
    CapabilityManifest, CanonicalSchemaRef, DigestHex, EffectClass, ExactVersionRef,
    JAVASCRIPT_HOST_PROTOCOL_VERSION, JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile,
    JavaScriptEntrypointMetadata, LocalizedMetadata, MINIMUM_NODE_MAJOR,
    PLUGIN_N1_SCHEMA_VERSION, PLUGIN_PACKAGE_PROFILE_VERSION, PackageContributions, PackageId,
    PackageManifest, PlatformConstraint, PluginPackageArtifactV1, PluginPackageV1Manifest,
    RuntimeTarget, StrictJsonValue, ToolPresentationKind, VersionString,
    CANDIDATE_TEST_CONTRACT_VERSION, CandidateTestOutcome, CandidateTestProvenance,
    PluginHostCommitFence, canonical_json_bytes, digest_bytes,
};
use nomifun_api_types::{ImportPluginRequest, PluginImportKindDto, PluginShareSourceDto, SharePluginRequest};
use nomifun_db::installation_owner_id;
use nomifun_js_authoring::{
    ExactDependencyLock, NeverCancel, NpmResolverIdentity, PluginLanguage, PluginProjectId,
    PluginScaffoldRequest, SourceScope, SourceStore, SourceStoreLimits, UserId,
};
use nomifun_plugin_platform::{ArtifactStoreLimits, PluginArtifactStore};
use nomifun_plugin_service::{PluginShareBundleFilesystem, PluginShareExport};
use nomifun_plugin_service::{
    DbPluginRepositoryAdapter, FsPluginArtifactStore, FsPluginMountDataStore,
    FsPluginSourceStore, PluginApplicationService, PluginHostCoordinator,
    PluginOperationCancellation, PluginRegistryPublisher, PluginServiceDependencies,
    PluginServiceError, PluginServicePaths, UnconfiguredPluginBuildExecutor,
    UnconfiguredPluginCandidateTestExecutor,
};
use nomifun_plugin_platform::OwnerMutationCoordinator;
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn package_artifact(main: &[u8], lock_digest: DigestHex) -> PluginPackageArtifactV1 {
    let package = ExactVersionRef {
        id: PackageId::from("example.plugin-share"),
        version: VersionString::from("1.0.0"),
    };
    let input_schema = StrictJsonValue(serde_json::json!({
        "additionalProperties": false,
        "type": "object"
    }));
    let output_schema = StrictJsonValue(serde_json::json!({"type":"object"}));
    let input_ref = CanonicalSchemaRef::from(format!(
        "schema://example/share-input@1#{}",
        nomifun_agent_contracts::digest_payload(&input_schema.0)
            .unwrap()
            .as_ref()
    ));
    let output_ref = CanonicalSchemaRef::from(format!(
        "schema://example/share-output@1#{}",
        nomifun_agent_contracts::digest_payload(&output_schema.0)
            .unwrap()
            .as_ref()
    ));
    let capability = CapabilityManifest {
        id: CapabilityId::from("example.plugin-share.echo"),
        contribution_id: "capability:example.plugin-share.echo".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: LocalizedMetadata {
            name: "Share Echo".into(),
            description: "Share fixture action.".into(),
            localized_names: Default::default(),
            localized_descriptions: Default::default(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: nomifun_agent_contracts::capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("example.plugin-share.echo.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::ReadLocal,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    let manifest = PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            package_id: package.id,
            package_version: package.version,
            display: LocalizedMetadata {
                name: "Plugin Share".into(),
                description: "Share Bundle fixture.".into(),
                localized_names: Default::default(),
                localized_descriptions: Default::default(),
            },
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: JavaScriptEntrypointMetadata {
                normalized_relative_path: "main.mjs".into(),
                module_digest: digest_bytes(main),
                host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            }
            .into(),
            contributions: PackageContributions {
                capabilities: vec![capability],
                ..Default::default()
            },
        },
        schemas: BTreeMap::from([(input_ref, input_schema), (output_ref, output_schema)]),
        supported_targets: BTreeSet::from([RuntimeTarget::from(
            "x86_64-pc-windows-msvc",
        )]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: lock_digest,
        credential_slots: Vec::new(),
    };
    PluginPackageArtifactV1::new(
        ArtifactId::from(id()),
        manifest,
        vec![ArtifactFileDigest {
            normalized_relative_path: "main.mjs".into(),
            digest: digest_bytes(main),
            size_bytes: main.len() as u64,
        }],
    )
    .unwrap()
}

#[derive(Default)]
struct NoopHost;

#[async_trait]
impl PluginHostCoordinator for NoopHost {
    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        Ok(PluginHostCommitFence::NotResident)
    }
}

#[derive(Default)]
struct NoopRegistry;

#[async_trait]
impl PluginRegistryPublisher for NoopRegistry {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        _mount: &nomifun_db::PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct NoopCancellation;

#[async_trait]
impl PluginOperationCancellation for NoopCancellation {
    async fn cancel(
        &self,
        _operation: &nomifun_db::ProductOperationRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

fn write_package(root: &Path, artifact: &PluginPackageArtifactV1, main: &[u8]) {
    fs::create_dir(root).unwrap();
    fs::write(
        root.join("manifest.json"),
        canonical_json_bytes(&artifact.manifest).unwrap(),
    )
    .unwrap();
    fs::write(root.join("main.mjs"), main).unwrap();
}

#[test]
fn plugin_share_bundle_round_trips_exact_artifact_source_and_lock() {
    let temp = tempfile::tempdir().unwrap();
    let source_store =
        SourceStore::new(temp.path().join("source-store"), SourceStoreLimits::default()).unwrap();
    let scope = SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();
    let project = source_store
        .create_plugin_project(
            scope.clone(),
            &PluginScaffoldRequest {
                package_id: "example.plugin-share".into(),
                package_version: "1.0.0".into(),
                display_name: "Plugin Share".into(),
                description: "Share Bundle fixture.".into(),
                language: PluginLanguage::JavaScript,
            },
            &NeverCancel,
        )
        .unwrap();
    let lock = ExactDependencyLock::empty(
        project.capture().dependency_requests(),
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
    )
    .unwrap();
    let lock_digest = source_store
        .write_initial_dependency_lock(
            &scope,
            project.capture().snapshot(),
            &lock,
            &NeverCancel,
        )
        .unwrap();
    let source = source_store
        .export_project_source(&scope, &NeverCancel)
        .unwrap();
    let main = b"export async function activate() { return { capabilities: {} }; }\n";
    let artifact = package_artifact(main, lock_digest);
    let incoming = temp.path().join("incoming-package");
    fs::create_dir(&incoming).unwrap();
    fs::write(
        incoming.join("manifest.json"),
        canonical_json_bytes(&ArtifactEnvelope::new(artifact.manifest.payload.clone()).unwrap())
            .unwrap(),
    )
    .unwrap();
    fs::write(incoming.join("main.mjs"), main).unwrap();
    let artifact_store = PluginArtifactStore::new(
        temp.path().join("artifact-store"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let stored = artifact_store
        .import_directory(&incoming, &nomifun_plugin_platform::NeverCancel)
        .unwrap()
        .stored;
    let destination = temp.path().join("plugin-share");
    let filesystem = PluginShareBundleFilesystem;
    let manifest = filesystem
        .export(
            PluginShareExport {
                originating_project_id: Some(scope.project_id().as_ref()),
                artifact: &stored,
                source: Some(&source),
                test_provenance: None,
            },
            &destination,
        )
        .unwrap();
    let imported = filesystem.import(&destination).unwrap();

    assert_eq!(imported.manifest, manifest);
    assert_eq!(imported.artifact.artifact_digest, stored.artifact.artifact_digest);
    assert_eq!(
        imported.source.as_ref().unwrap().snapshot(),
        source.snapshot()
    );
    assert_eq!(
        imported.source.as_ref().unwrap().dependency_lock(),
        source.dependency_lock()
    );
    assert_eq!(
        imported.bundle_digest,
        digest_bytes(&fs::read(destination.join("bundle.json")).unwrap())
            .as_ref()
            .to_owned()
    );
}

#[test]
fn plugin_share_bundle_rejects_artifact_tamper_and_extra_user_data() {
    let temp = tempfile::tempdir().unwrap();
    let package_root = temp.path().join("incoming-package");
    fs::create_dir(&package_root).unwrap();
    let main = b"export async function activate() { return { capabilities: {} }; }\n";
    let empty_requests = nomifun_js_authoring::DependencyRequestSet::new([]).unwrap();
    let lock = ExactDependencyLock::empty(
        &empty_requests,
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
    )
    .unwrap();
    let artifact = package_artifact(main, lock.digest().unwrap());
    fs::write(
        package_root.join("manifest.json"),
        canonical_json_bytes(&artifact.manifest).unwrap(),
    )
    .unwrap();
    fs::write(package_root.join("main.mjs"), main).unwrap();
    let store = PluginArtifactStore::new(
        temp.path().join("artifact-store"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let stored = store
        .import_directory(&package_root, &nomifun_plugin_platform::NeverCancel)
        .unwrap()
        .stored;
    let filesystem = PluginShareBundleFilesystem;
    let tampered = temp.path().join("tampered");
    filesystem
        .export(
            PluginShareExport {
                originating_project_id: None,
                artifact: &stored,
                source: None,
                test_provenance: None,
            },
            &tampered,
        )
        .unwrap();
    fs::write(tampered.join("artifact/package/main.mjs"), b"tampered").unwrap();
    assert!(filesystem.import(&tampered).is_err());

    let extra = temp.path().join("extra");
    filesystem
        .export(
            PluginShareExport {
                originating_project_id: None,
                artifact: &stored,
                source: None,
                test_provenance: None,
            },
            &extra,
        )
        .unwrap();
    fs::write(extra.join("credentials.json"), b"{}").unwrap();
    assert!(filesystem.import(&extra).is_err());

    let extra_directory = temp.path().join("extra-directory");
    filesystem
        .export(
            PluginShareExport {
                originating_project_id: None,
                artifact: &stored,
                source: None,
                test_provenance: None,
            },
            &extra_directory,
        )
        .unwrap();
    fs::create_dir(extra_directory.join("empty-user-data")).unwrap();
    assert!(filesystem.import(&extra_directory).is_err());
}

#[tokio::test]
async fn share_application_imports_new_editable_project_and_reexports_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let source_root = temp.path().join("source-store");
    let source_store = SourceStore::new(&source_root, SourceStoreLimits::default()).unwrap();
    let source_scope = SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();
    let source_project = source_store
        .create_plugin_project(
            source_scope.clone(),
            &PluginScaffoldRequest {
                package_id: "example.plugin-share".into(),
                package_version: "1.0.0".into(),
                display_name: "Plugin Share".into(),
                description: "Share Bundle fixture.".into(),
                language: PluginLanguage::JavaScript,
            },
            &NeverCancel,
        )
        .unwrap();
    let lock = ExactDependencyLock::empty(
        source_project.capture().dependency_requests(),
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
    )
    .unwrap();
    let lock_digest = source_store
        .write_initial_dependency_lock(
            &source_scope,
            source_project.capture().snapshot(),
            &lock,
            &NeverCancel,
        )
        .unwrap();
    let source = source_store
        .export_project_source(&source_scope, &NeverCancel)
        .unwrap();
    let main = b"export async function activate() { return { capabilities: {} }; }\n";
    let artifact = package_artifact(main, lock_digest);
    let package_root = temp.path().join("incoming-package");
    write_package(&package_root, &artifact, main);
    let artifact_root = temp.path().join("artifact-store");
    let artifact_store =
        PluginArtifactStore::new(&artifact_root, ArtifactStoreLimits::default()).unwrap();
    let stored = artifact_store
        .import_directory(&package_root, &nomifun_plugin_platform::NeverCancel)
        .unwrap()
        .stored;
    let incoming_bundle = temp.path().join("incoming-share");
    PluginShareBundleFilesystem
        .export(
            PluginShareExport {
                originating_project_id: Some(source_scope.project_id().as_ref()),
                artifact: &stored,
                source: Some(&source),
                test_provenance: Some(CandidateTestProvenance {
                    outcome: CandidateTestOutcome::Passed,
                    candidate_digest: DigestHex::from("c".repeat(64)),
                    runtime_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
                    runtime_executable_digest: DigestHex::from("d".repeat(64)),
                    host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                    javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                    test_contract_version: CANDIDATE_TEST_CONTRACT_VERSION.into(),
                }),
            },
            &incoming_bundle,
        )
        .unwrap();
    let bundle_digest = digest_bytes(&fs::read(incoming_bundle.join("bundle.json")).unwrap())
        .as_ref()
        .to_owned();

    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner_user_id = installation_owner_id(database.pool()).await.unwrap();
    let repository = Arc::new(DbPluginRepositoryAdapter::new(database.pool().clone()));
    let service = PluginApplicationService::new(PluginServiceDependencies {
        repository,
        artifacts: Arc::new(
            FsPluginArtifactStore::new(&artifact_root, ArtifactStoreLimits::default()).unwrap(),
        ),
        host: Arc::new(NoopHost),
        registry: Arc::new(NoopRegistry),
        mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
        builder: Arc::new(UnconfiguredPluginBuildExecutor),
        tester: Arc::new(UnconfiguredPluginCandidateTestExecutor),
        operation_cancellation: Arc::new(NoopCancellation),
        source_store: Arc::new(
            FsPluginSourceStore::new(&source_root, SourceStoreLimits::default()).unwrap(),
        ),
        data_store: Arc::new(FsPluginMountDataStore::new(temp.path()).unwrap()),
        paths: PluginServicePaths {
            mount_data_relative_root: "plugin-mount-data".into(),
        },
    });
    let imported = service
        .import_prebuilt(
            &owner_user_id,
            ImportPluginRequest {
                expected_library_revision: 0,
                import_kind: PluginImportKindDto::ShareBundle,
                source_path: incoming_bundle.display().to_string(),
                expected_bundle_or_artifact_digest: bundle_digest,
                target_project_id: None,
                expected_project_revision: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(imported.summary.build_generation, 1);
    assert!(imported.source_snapshot_digest.is_some());
    assert!(imported.dependency_lock_digest.is_some());
    let ready = imported.ready.as_ref().unwrap();
    assert_eq!(
        ready.test.status,
        nomifun_api_types::PluginCandidateTestStatusDto::NotRun
    );
    assert_eq!(
        ready.imported_test_provenance.as_ref().unwrap().outcome,
        nomifun_api_types::PluginCandidateTestStatusDto::Passed
    );

    let reexport = temp.path().join("reexported-share");
    let operation = service
        .export_share(
            &owner_user_id,
            SharePluginRequest {
                project_id: imported.summary.project_id.clone(),
                expected_project_revision: imported.summary.project_revision,
                source: PluginShareSourceDto::ReadyCandidate,
                candidate_id: Some(ready.candidate.candidate_id.clone()),
                expected_candidate_digest: Some(ready.candidate.candidate_digest.clone()),
                mount_id: None,
                expected_mount_revision: None,
                expected_target_digest: None,
                destination_path: reexport.display().to_string(),
                include_source: true,
            },
        )
        .await
        .unwrap();
    let reexported = PluginShareBundleFilesystem.import(&reexport).unwrap();
    assert_eq!(
        operation.result_artifact_digests.get("share_bundle"),
        Some(&reexported.bundle_digest)
    );
    assert!(reexported.source.is_some());
    assert_eq!(
        reexported.manifest.test_provenance.unwrap().candidate_digest,
        DigestHex::from("c".repeat(64))
    );
}
