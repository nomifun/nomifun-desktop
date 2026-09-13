use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, CapabilityActionDescriptor, CapabilityConsumer,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest, EffectClass,
    ExactVersionRef, LocalizedMetadata, PackageContributions, PlatformConstraint, PluginProjectId,
    RuntimeTarget, StrictJsonValue, ToolPresentationKind, UserId, capability_surface_declarations,
};
use nomifun_api_types::{
    BuildPluginProjectRequest, CreatePluginProjectRequest, PluginProjectLanguageDto,
};
use nomifun_db::PluginProjectRow;
use nomifun_js_authoring::{
    FixedPluginPacker, NeverCancel, NodeBuildHost, PluginSourceManifest, SourceScope,
    SourceStoreLimits,
};
use nomifun_plugin_platform::ArtifactStoreLimits;
use nomifun_plugin_platform::application::{
    FsPluginArtifactStore, FsPluginBuildExecutor, FsPluginSourceStore, PluginBuildExecutor,
    PluginSourceStorePort,
};
use serde_json::json;
use uuid::Uuid;

fn node_executable() -> PathBuf {
    if let Some(value) = std::env::var_os("NODE_EXECUTABLE") {
        return PathBuf::from(value);
    }
    let path = std::env::var_os("PATH").expect("PATH is available");
    std::env::split_paths(&path)
        .flat_map(|directory| {
            #[cfg(windows)]
            let names = ["node.exe", "node"];
            #[cfg(not(windows))]
            let names = ["node", "node"];
            names.map(move |name| directory.join(name))
        })
        .find(|candidate| candidate.is_file())
        .expect("Node 24+ is required for Plugin build executor tests")
        .canonicalize()
        .expect("Node path is canonical")
}

fn runtime_target() -> RuntimeTarget {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return RuntimeTarget::from("x86_64-pc-windows-msvc");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return RuntimeTarget::from("aarch64-apple-darwin");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return RuntimeTarget::from("x86_64-unknown-linux-gnu");
    #[allow(unreachable_code)]
    RuntimeTarget::from("unsupported")
}

fn declare_capability(source_root: &Path) {
    let manifest_path = source_root.join("nomifun.plugin.json");
    let manifest =
        PluginSourceManifest::from_canonical_bytes(&fs::read(&manifest_path).unwrap()).unwrap();
    let package = ExactVersionRef {
        id: manifest.package_id().clone(),
        version: manifest.package_version().clone(),
    };
    let input_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {"input": {}},
        "required": ["input"],
        "type": "object"
    }));
    let output_schema = StrictJsonValue(json!({}));
    let input_ref = CanonicalSchemaRef::from(format!(
        "schema://example.executor/input@1#{}",
        nomifun_agent_contracts::digest_payload(&input_schema.0)
            .unwrap()
            .as_ref()
    ));
    let output_ref = CanonicalSchemaRef::from(format!(
        "schema://example.executor/output@1#{}",
        nomifun_agent_contracts::digest_payload(&output_schema.0)
            .unwrap()
            .as_ref()
    ));
    let capability = CapabilityManifest {
        id: CapabilityId::from("example.executor.echo"),
        contribution_id: "capability:example.executor.echo".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package,
        display: LocalizedMetadata {
            name: "Echo".into(),
            description: "Return the exact input.".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type": "object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("example.executor.echo.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::Pure,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    let manifest = manifest
        .with_contributions_and_schemas(
            PackageContributions {
                capabilities: vec![capability],
                ..Default::default()
            },
            BTreeMap::from([
                (input_ref, input_schema),
                (output_ref, output_schema),
            ]),
        )
        .unwrap();
    fs::write(manifest_path, manifest.canonical_bytes().unwrap()).unwrap();
    fs::write(
        source_root.join("src/main.ts"),
        r#"type ActivationInput = Readonly<{ input: unknown }>;

export async function activate() {
  return {
    capabilities: {
      "capability:example.executor.echo": {
        async invoke({ input }: ActivationInput) {
          return input;
        },
      },
    },
  };
}
"#,
    )
    .unwrap();
}

#[tokio::test]
async fn filesystem_build_executor_publishes_verified_node_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let source_store = FsPluginSourceStore::new(
        temp.path().join("authoring"),
        SourceStoreLimits::default(),
    )
    .unwrap();
    let artifact_store = FsPluginArtifactStore::new(
        temp.path().join("artifact-store"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let owner = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    let created = source_store
        .create_project(
            &owner,
            &project_id,
            &CreatePluginProjectRequest {
                expected_library_revision: 0,
                package_id: "example.executor".into(),
                package_version: "0.1.0".into(),
                display_name: "Executor".into(),
                description: "Real Build Executor fixture.".into(),
                language: PluginProjectLanguageDto::TypeScript,
                linked_mount_id: None,
                expected_linked_mount_revision: None,
                expected_linked_target_digest: None,
            },
        )
        .await
        .unwrap();
    let scope = SourceScope::new(
        UserId::from(owner.clone()),
        PluginProjectId::from(project_id.clone()),
    )
    .unwrap();
    let project = source_store.store().load_project(&scope).unwrap();
    declare_capability(project.source_root());
    let source = source_store.store().snapshot(&scope, &NeverCancel).unwrap();
    let lock = source_store
        .store()
        .load_dependency_lock(&scope, &NeverCancel)
        .unwrap();
    let lock_digest = lock.digest().unwrap();
    let executor = FsPluginBuildExecutor::new(
        &source_store,
        &artifact_store,
        FixedPluginPacker::new(
            NodeBuildHost::new(node_executable(), Duration::from_secs(20)).unwrap(),
        ),
        runtime_target(),
    )
    .unwrap();
    let project_row = PluginProjectRow {
        id: 1,
        project_id: project_id.clone(),
        owner_user_id: owner,
        package_id: "example.executor".into(),
        display_name: "Executor".into(),
        description: "Real Build Executor fixture.".into(),
        apply_mode: "ask_before_apply".into(),
        auto_apply_mount_id: None,
        auto_apply_authorization_revision: 0,
        auto_apply_authorized_at: None,
        managed_source_path: Some(created.managed_relative_path),
        source_head_digest: Some(source.snapshot().digest().as_ref().to_owned()),
        dependency_lock_digest: Some(lock_digest.as_ref().to_owned()),
        build_generation: 1,
        linked_mount_id: None,
        ready_candidate_id: None,
        created_at: 1,
        updated_at: 1,
    };
    let output = executor
        .build(
            &Uuid::now_v7().to_string(),
            &project_row,
            &BuildPluginProjectRequest {
                project_id,
                expected_project_revision: 1,
                expected_build_generation: 1,
                expected_source_snapshot_digest: source.snapshot().digest().as_ref().to_owned(),
                expected_dependency_lock_digest: lock_digest.as_ref().to_owned(),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        output.artifact.manifest.payload.package.package_id.as_ref(),
        "example.executor"
    );
    assert_eq!(
        output.source_snapshot_digest,
        source.snapshot().digest().as_ref()
    );
    assert_eq!(output.dependency_lock_digest, lock_digest.as_ref());
    let stored = artifact_store
        .store()
        .load(&output.artifact.artifact_digest)
        .unwrap();
    assert_eq!(stored.artifact, output.artifact);
    assert!(stored.package_root.join("main.mjs").is_file());
    assert_eq!(
        fs::read_dir(source_store.store().managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn filesystem_build_executor_rejects_overlapping_managed_roots() {
    let temp = tempfile::tempdir().unwrap();
    let artifact_store = FsPluginArtifactStore::new(
        temp.path().join("managed"),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let source_store = FsPluginSourceStore::new(
        artifact_store.store().managed_root().join("authoring"),
        SourceStoreLimits::default(),
    )
    .unwrap();
    let result = FsPluginBuildExecutor::new(
        &source_store,
        &artifact_store,
        FixedPluginPacker::new(
            NodeBuildHost::new(node_executable(), Duration::from_secs(20)).unwrap(),
        ),
        runtime_target(),
    );
    assert!(result.is_err());
}
