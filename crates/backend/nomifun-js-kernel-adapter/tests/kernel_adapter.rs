use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nomifun_agent_contracts::{
    ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload,
    AgentSessionId, ArtifactFileDigest, ArtifactId, CapabilityActionDescriptor,
    CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind,
    CapabilityManifest, CapabilityRef, CapabilitySelection, CanonicalSchemaRef,
    CorrelationId, DigestHex, EffectClass, IdempotencyKey,
    JAVASCRIPT_HOST_PROTOCOL_VERSION, JAVASCRIPT_SDK_CONTRACT_VERSION,
    JavaScriptBuildProfile, JavaScriptEntrypointMetadata, LocalizedMetadata,
    MINIMUM_NODE_MAJOR, OperationId, PLUGIN_N1_SCHEMA_VERSION,
    PLUGIN_PACKAGE_PROFILE_VERSION, PackageContributions, PackageId,
    PackageManifest, PackageRef, PlatformConstraint, PluginMountId,
    PluginPackageArtifactV1, PluginPackageV1Manifest, PluginSourceKind,
    PresetRevisionRef, PrincipalRef, ResourceBindingId, ResourceId, ResourceKind,
    RuntimeProfileKind, RuntimeTarget, ScopeKey, StrictJsonValue,
    ToolPresentationKind, TypedResourceBinding, UserId, ValidatedPluginConfig,
    VersionString, capability_surface_declarations, digest_payload,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler, CapabilityAccessRequest, CapabilityInvocationRequest,
    CapabilityOperationRequest, CompileRequest, CompilerEnvironment,
    InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, PluginStatePersistence,
    SessionCapabilityState,
};
use nomifun_js_host::{
    ExtensionHostSupervisor, JavaScriptHostConfig, JavaScriptHostLimits,
    JavaScriptHostState,
};
use nomifun_js_kernel_adapter::{JsKernelPluginAdapter, PluginPackageInput};
use nomifun_js_runtime::{NodeProbeCandidate, NodeRuntimeResolver};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const VERSION: &str = "1.0.0";
const TOOL_ID: &str = "fixture.tool";
const TOOL_ACTION: &str = "fixture.tool.invoke";
const UI_TOOL_ID: &str = "fixture.ui";
const UI_TOOL_ACTION: &str = "fixture.ui.invoke";
const RELEASE_COUNT_ACTION: &str = "fixture.release_count";
const CONTEXT_ID: &str = "fixture.context";
const RESOURCE_ID: &str = "fixture.resource";
const RESOURCE_KIND: &str = "fixture.resource-kind";

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(path)
}

fn sha256(bytes: &[u8]) -> DigestHex {
    DigestHex::from(hex::encode(Sha256::digest(bytes)))
}

fn fixture_schema() -> StrictJsonValue {
    StrictJsonValue(json!({
        "additionalProperties": true,
        "type": "object"
    }))
}

fn fixture_schema_ref(name: &str) -> CanonicalSchemaRef {
    let schema = fixture_schema();
    CanonicalSchemaRef::from(format!(
        "schema://fixture/{name}@1#{}",
        digest_payload(&schema.0).unwrap().as_ref()
    ))
}

fn fixture_schemas() -> BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
    [
        "tool-input",
        "tool-output",
        "release-count-input",
        "release-count-output",
        "context",
    ]
    .into_iter()
    .map(|name| (fixture_schema_ref(name), fixture_schema()))
    .collect()
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn capability(
    package: &PackageRef,
    id: &str,
    contribution_id: &str,
    kind: CapabilityKind,
) -> CapabilityManifest {
    let contributions = match kind {
        CapabilityKind::Tool => CapabilityContributions {
            actions: vec![
                CapabilityActionDescriptor {
                    action_id: ActionId::from(if id == UI_TOOL_ID {
                        UI_TOOL_ACTION
                    } else {
                        TOOL_ACTION
                    }),
                    input_schema: fixture_schema_ref("tool-input"),
                    output_schema: fixture_schema_ref("tool-output"),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::FunctionTool,
                },
                CapabilityActionDescriptor {
                    action_id: ActionId::from(RELEASE_COUNT_ACTION),
                    input_schema: fixture_schema_ref("release-count-input"),
                    output_schema: fixture_schema_ref("release-count-output"),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::Hidden,
                },
            ]
            .into_iter()
            .filter(|action| {
                id != UI_TOOL_ID || action.action_id.as_ref() == UI_TOOL_ACTION
            })
            .collect(),
            ..Default::default()
        },
        CapabilityKind::ContextContributor => CapabilityContributions {
            context_schema_refs: vec![fixture_schema_ref("context")],
            ..Default::default()
        },
        CapabilityKind::ResourceProvider => CapabilityContributions {
            resource_kinds: BTreeSet::from([ResourceKind::from(RESOURCE_KIND)]),
            ..Default::default()
        },
        _ => unreachable!("N1 fixture supports only executable capability kinds"),
    };
    CapabilityManifest {
        id: CapabilityId::from(id),
        contribution_id: contribution_id.into(),
        version: VersionString::from(VERSION),
        kind,
        package: package.clone(),
        display: display(id, "JavaScript Kernel adapter fixture."),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            if id == UI_TOOL_ID {
                vec![CapabilityConsumer::Ui]
            } else if id == TOOL_ID {
                vec![CapabilityConsumer::Agent, CapabilityConsumer::Gateway]
            } else {
                vec![CapabilityConsumer::Agent]
            },
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type": "object"})),
        contributions,
    }
}

fn artifact(main: &[u8]) -> PluginPackageArtifactV1 {
    artifact_with_variant(main, None)
}

fn artifact_with_variant(
    main: &[u8],
    variant: Option<char>,
) -> PluginPackageArtifactV1 {
    let package = PackageRef {
        id: PackageId::from("fixture.javascript"),
        version: VersionString::from(VERSION),
    };
    let manifest = PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            package_id: package.id.clone(),
            package_version: package.version.clone(),
            display: display(
                "JavaScript Adapter Fixture",
                "Exercises direct Tool, Context, and Resource exports.",
            ),
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
                capabilities: vec![
                    capability(
                        &package,
                        TOOL_ID,
                        "fixture.tool.contribution",
                        CapabilityKind::Tool,
                    ),
                    capability(
                        &package,
                        UI_TOOL_ID,
                        "fixture.ui.contribution",
                        CapabilityKind::Tool,
                    ),
                    capability(
                        &package,
                        CONTEXT_ID,
                        "fixture.context.contribution",
                        CapabilityKind::ContextContributor,
                    ),
                    capability(
                        &package,
                        RESOURCE_ID,
                        "fixture.resource.contribution",
                        CapabilityKind::ResourceProvider,
                    ),
                ],
                ..Default::default()
            },
        },
        schemas: fixture_schemas(),
        supported_targets: BTreeSet::from([RuntimeTarget::from(
            "x86_64-pc-windows-msvc",
        )]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: DigestHex::from("b".repeat(64)),
        credential_slots: Vec::new(),
    };
    let mut files = vec![ArtifactFileDigest {
        normalized_relative_path: "main.mjs".into(),
        digest: sha256(main),
        size_bytes: u64::try_from(main.len()).unwrap(),
    }];
    if let Some(variant) = variant {
        files.push(ArtifactFileDigest {
            normalized_relative_path: "resources/variant.txt".into(),
            digest: DigestHex::from(variant.to_string().repeat(64)),
            size_bytes: 1,
        });
    }
    PluginPackageArtifactV1::new(
        ArtifactId::from("fixture-artifact"),
        manifest,
        files,
    )
    .unwrap()
}

fn node_executable() -> PathBuf {
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
        .expect("Node 22+ is required for JavaScript adapter tests")
        .canonicalize()
        .unwrap()
}

async fn host() -> Arc<ExtensionHostSupervisor> {
    let executable = node_executable();
    let probe = NodeRuntimeResolver::default()
        .probe(&NodeProbeCandidate::new(
            nomifun_agent_contracts::NodeRuntimeSourceKind::ProcessPath,
            executable.clone(),
        ))
        .await;
    let runtime = probe
        .fingerprint
        .unwrap_or_else(|| panic!("Node probe failed: {:?}", probe.error_code));
    Arc::new(
        ExtensionHostSupervisor::new(JavaScriptHostConfig {
            node_executable: executable,
            runtime,
            host_module: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../nomifun-js-host/assets/extension-host.mjs")
                .canonicalize()
                .unwrap(),
            limits: JavaScriptHostLimits {
                hello_timeout: Duration::from_secs(5),
                request_timeout: Duration::from_secs(5),
                shutdown_timeout: Duration::from_secs(5),
                max_frame_bytes: 1024 * 1024,
                command_queue_capacity: 32,
                ..JavaScriptHostLimits::default()
            },
        })
        .unwrap(),
    )
}

fn revision(
    owner: &PrincipalRef,
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
) -> AgentPresetRevision {
    let selection = |id: &str, actions: &[&str]| CapabilitySelection {
        capability: CapabilityRef {
            id: CapabilityId::from(id),
            version: VersionString::from(VERSION),
        },
        action_allowlist: actions
            .iter()
            .map(|action| ActionId::from(*action))
            .collect(),
    };
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from("fixture.javascript.preset"),
            revision: 1,
            revision_digest: DigestHex::from(""),
        },
        payload: AgentPresetRevisionPayload {
            schema_version: VersionString::from(VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![
                selection(TOOL_ID, &[TOOL_ACTION, RELEASE_COUNT_ACTION]),
                selection(CONTEXT_ID, &[]),
                selection(RESOURCE_ID, &[]),
            ],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "JavaScript adapter fixture".into(),
            instructions: "Exercise direct typed exports.".into(),
            starter_prompts: Vec::new(),
        },
        contribution_locks: [TOOL_ID, CONTEXT_ID, RESOURCE_ID]
            .into_iter()
            .map(|id| {
                materialized
                    .capability(&CapabilityId::from(id))
                    .unwrap()
                    .contribution_lock
                    .clone()
            })
            .collect(),
        created_by: UserId::from(owner.principal_id.clone()),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

fn environment(registry_digest: DigestHex) -> CompilerEnvironment {
    CompilerEnvironment {
        resolver_version: VersionString::from(VERSION),
        required_runtime_protocol_version: VersionString::from(VERSION),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
        available_runtime_features: BTreeSet::new(),
        installation_role_bindings: BTreeMap::new(),
        canonical_schema_manifest_digest: DigestHex::from("schema-manifest"),
        target_contribution_manifest_digest: registry_digest,
        host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
        host_surface: "desktop".into(),
        availability_evidence_revision: "js-adapter-fixture".into(),
    }
}

fn access(
    snapshot: &nomifun_agent_kernel::CompiledSnapshot,
    active_generation: u64,
    owner: &PrincipalRef,
    capability_id: &str,
) -> CapabilityAccessRequest {
    let capability_id = CapabilityId::from(capability_id);
    CapabilityAccessRequest {
        principal: owner.clone(),
        session_owner: owner.clone(),
        agent_session_id: AgentSessionId::from("fixture-session"),
        operation_id: OperationId::from(format!(
            "fixture-access:{}",
            capability_id.as_ref()
        )),
        correlation_id: CorrelationId::from(format!(
            "fixture-correlation:{}",
            capability_id.as_ref()
        )),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: active_generation,
        resource_binding_ids: snapshot
            .policy(&capability_id)
            .unwrap()
            .resource_binding_ids
            .clone(),
        capability_id,
        state_scope_key: ScopeKey::from("session:fixture-session"),
    }
}

fn compile_snapshot(
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
    owner: &PrincipalRef,
) -> nomifun_agent_kernel::CompiledSnapshot {
    AgentPresetCompiler::compile(
        materialized,
        &environment(materialized.registry_digest.clone()),
        CompileRequest {
            miniapp_capabilities: Vec::new(),
            revision: revision(owner, materialized),
            principal: owner.clone(),
            scene: "fixture".into(),
            surface: "desktop".into(),
            audience: "test".into(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("fixture-resolve"),
        },
    )
    .unwrap()
    .with_target_resource_bindings(
        owner,
        vec![TypedResourceBinding {
            binding_id: ResourceBindingId::from("fixture-binding"),
            resource_kind: ResourceKind::from(RESOURCE_KIND),
            resource_id: ResourceId::from("fixture-resource"),
            owner_id: owner.principal_id.clone(),
            operations: BTreeSet::from(["acquire".into()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "path".into(),
                "fixture".into(),
            )]),
        }],
    )
    .unwrap()
}

#[tokio::test]
async fn one_kernel_registry_dispatches_javascript_tool_context_and_resource() {
    let main_path = fixture("main.mjs").canonicalize().unwrap();
    let main = tokio::fs::read(&main_path).await.unwrap();
    let package_root = main_path.parent().unwrap().to_path_buf();
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("plugin-data");
    tokio::fs::create_dir_all(&data_dir).await.unwrap();
    let artifact = artifact(&main);
    let config_schema = artifact.manifest.payload.package.config_schema.clone();
    let adapter = JsKernelPluginAdapter::new(PluginPackageInput {
        artifact,
        mount_id: PluginMountId::from("fixture-javascript-mount"),
        package_root,
        config: ValidatedPluginConfig {
            schema_digest: digest_payload(&config_schema).unwrap(),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        credential_bindings: Vec::new(),
        data_dir,
    })
    .unwrap();
    let host = host().await;
    assert_eq!(host.process_count(), 0);

    let registry = KernelRegistry::new(
        MaterializationPolicy {
            host_contract_version: VersionString::from(VERSION),
            available_runtime_features: BTreeSet::new(),
            allowed_sources: BTreeSet::from([PluginSourceKind::ManagedLocal]),
        },
        Arc::new(InMemoryPluginStatePersistence::new())
            as Arc<dyn PluginStatePersistence>,
    )
    .unwrap();
    let materialized = registry
        .replace_all(vec![adapter.registration(host.clone()).unwrap()])
        .unwrap();
    assert_eq!(host.process_count(), 0);
    for capability_id in [TOOL_ID, CONTEXT_ID, RESOURCE_ID] {
        let capability = materialized
            .capability(&CapabilityId::from(capability_id))
            .unwrap();
        assert_eq!(
            capability.contribution_lock.source_kind,
            nomifun_agent_contracts::ContributionSourceKind::PluginMount
        );
        assert_eq!(
            capability.contribution_lock.mount_id.as_ref(),
            Some(&PluginMountId::from("fixture-javascript-mount"))
        );
        assert_eq!(
            capability.target_artifact_digest,
            adapter.artifact().artifact_digest
        );
        assert_eq!(
            capability.contribution_lock.contract_digest,
            capability.schema_digest
        );
        assert_eq!(capability.manifest.package, adapter.manifest().package_ref());
    }

    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let compiled = compile_snapshot(&materialized, &owner);
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();

    let context = registry
        .contribute_context(
            &compiled,
            &active,
            access(&compiled, active.generation, &owner, CONTEXT_ID),
        )
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(
        context.0["schemaRef"],
        fixture_schema_ref("context").as_ref()
    );
    assert_eq!(
        context.0["contributionId"],
        "fixture.context.contribution"
    );

    let invoked = registry
        .invoke(
            &compiled,
            &active,
            CapabilityInvocationRequest {
                principal: owner.clone(),
                session_owner: owner.clone(),
                agent_session_id: AgentSessionId::from("fixture-session"),
                operation_id: OperationId::from("fixture-tool-operation"),
                idempotency_key: IdempotencyKey::from("fixture-tool-key"),
                correlation_id: CorrelationId::from("fixture-tool-correlation"),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from(TOOL_ID),
                action_id: ActionId::from(TOOL_ACTION),
                resource_binding_ids: BTreeSet::new(),
                state_scope_key: ScopeKey::from("session:fixture-session"),
                input: StrictJsonValue(json!({"value": 7})),
            },
        )
        .await
        .unwrap();
    assert_eq!(invoked.0["input"]["value"], 7);
    assert_eq!(invoked.0["contributionId"], "fixture.tool.contribution");

    let gateway_lock = materialized
        .capability(&CapabilityId::from(TOOL_ID))
        .unwrap()
        .operation_lock(CapabilityConsumer::Gateway);
    let gateway_invoked = registry
        .invoke_operation(CapabilityOperationRequest {
            principal: owner.clone(),
            operation_id: OperationId::from("fixture-gateway-operation"),
            idempotency_key: IdempotencyKey::from("fixture-gateway-key"),
            correlation_id: CorrelationId::from("fixture-gateway-correlation"),
            operation_lock: gateway_lock.clone(),
            action_id: ActionId::from(TOOL_ACTION),
            resource_bindings: Vec::new(),
            state_scope_key: ScopeKey::from("gateway:fixture"),
            input: StrictJsonValue(json!({"surface": "gateway"})),
        })
        .await
        .unwrap();
    assert_eq!(gateway_invoked.0["input"]["surface"], "gateway");
    assert!(
        registry
            .invoke_operation(CapabilityOperationRequest {
                principal: owner.clone(),
                operation_id: OperationId::from("fixture-agent-bypass-operation"),
                idempotency_key: IdempotencyKey::from("fixture-agent-bypass-key"),
                correlation_id: CorrelationId::from(
                    "fixture-agent-bypass-correlation",
                ),
                operation_lock: materialized
                    .capability(&CapabilityId::from(TOOL_ID))
                    .unwrap()
                    .operation_lock(CapabilityConsumer::Agent),
                action_id: ActionId::from(TOOL_ACTION),
                resource_bindings: Vec::new(),
                state_scope_key: ScopeKey::from("agent:bypass"),
                input: StrictJsonValue(json!({})),
            })
            .await
            .is_err()
    );

    let ui_capability = materialized
        .capability(&CapabilityId::from(UI_TOOL_ID))
        .unwrap();
    assert!(!ui_capability.manifest.supports_consumer(CapabilityConsumer::Agent));
    let ui_invoked = registry
        .invoke_operation(CapabilityOperationRequest {
            principal: owner.clone(),
            operation_id: OperationId::from("fixture-ui-operation"),
            idempotency_key: IdempotencyKey::from("fixture-ui-key"),
            correlation_id: CorrelationId::from("fixture-ui-correlation"),
            operation_lock: ui_capability.operation_lock(CapabilityConsumer::Ui),
            action_id: ActionId::from(UI_TOOL_ACTION),
            resource_bindings: Vec::new(),
            state_scope_key: ScopeKey::from("ui:fixture"),
            input: StrictJsonValue(json!({"surface": "ui"})),
        })
        .await
        .unwrap();
    assert_eq!(ui_invoked.0["contributionId"], "fixture.ui.contribution");

    let mut stale_lock = gateway_lock;
    stale_lock.target_artifact_digest = Some(DigestHex::from("f".repeat(64)));
    assert!(
        registry
            .invoke_operation(CapabilityOperationRequest {
                principal: owner.clone(),
                operation_id: OperationId::from("fixture-stale-operation"),
                idempotency_key: IdempotencyKey::from("fixture-stale-key"),
                correlation_id: CorrelationId::from("fixture-stale-correlation"),
                operation_lock: stale_lock,
                action_id: ActionId::from(TOOL_ACTION),
                resource_bindings: Vec::new(),
                state_scope_key: ScopeKey::from("gateway:fixture"),
                input: StrictJsonValue(json!({})),
            })
            .await
            .is_err()
    );

    let first = registry
        .acquire_resource(
            &compiled,
            &active,
            access(&compiled, active.generation, &owner, RESOURCE_ID),
        )
        .await
        .unwrap();
    let second = registry
        .acquire_resource(
            &compiled,
            &active,
            access(&compiled, active.generation, &owner, RESOURCE_ID),
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first.handle, &second.handle));
    assert_eq!(
        first.handle.identity().resource_id.as_ref(),
        "fixture-resource"
    );

    registry
        .release_resources(&ScopeKey::from("session:fixture-session"))
        .await
        .unwrap();
    let release_count = registry
        .invoke(
            &compiled,
            &active,
            CapabilityInvocationRequest {
                principal: owner.clone(),
                session_owner: owner,
                agent_session_id: AgentSessionId::from("fixture-session"),
                operation_id: OperationId::from("fixture-release-count"),
                idempotency_key: IdempotencyKey::from("fixture-release-count-key"),
                correlation_id: CorrelationId::from(
                    "fixture-release-count-correlation",
                ),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from(TOOL_ID),
                action_id: ActionId::from(RELEASE_COUNT_ACTION),
                resource_binding_ids: BTreeSet::new(),
                state_scope_key: ScopeKey::from("session:fixture-session"),
                input: StrictJsonValue(json!({})),
            },
        )
        .await
        .unwrap();
    assert_eq!(release_count.0["releaseCount"], 1);

    let JavaScriptHostState::Running { generation, .. } = host.state() else {
        panic!("typed JavaScript capability demand must start the shared Host");
    };
    host.stop_generation(generation).await.unwrap();
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn compatible_replace_does_not_reuse_an_old_artifact_resource_handle() {
    let main_path = fixture("main.mjs").canonicalize().unwrap();
    let main = tokio::fs::read(&main_path).await.unwrap();
    let package_root = main_path.parent().unwrap().to_path_buf();
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("plugin-data");
    tokio::fs::create_dir_all(&data_dir).await.unwrap();
    let first_artifact = artifact_with_variant(&main, Some('a'));
    let second_artifact = artifact_with_variant(&main, Some('c'));
    assert_ne!(
        first_artifact.artifact_digest,
        second_artifact.artifact_digest
    );
    let host = host().await;
    let registry = KernelRegistry::new(
        MaterializationPolicy {
            host_contract_version: VersionString::from(VERSION),
            available_runtime_features: BTreeSet::new(),
            allowed_sources: BTreeSet::from([PluginSourceKind::ManagedLocal]),
        },
        Arc::new(InMemoryPluginStatePersistence::new())
            as Arc<dyn PluginStatePersistence>,
    )
    .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };

    let make_adapter = |artifact: PluginPackageArtifactV1| {
        let config_schema = artifact.manifest.payload.package.config_schema.clone();
        JsKernelPluginAdapter::new(PluginPackageInput {
            artifact,
            mount_id: PluginMountId::from("fixture-javascript-mount"),
            package_root: package_root.clone(),
            config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).unwrap(),
                config_revision: 1,
                value: StrictJsonValue(json!({})),
            },
            credential_bindings: Vec::new(),
            data_dir: data_dir.clone(),
        })
        .unwrap()
    };

    let first_materialized = registry
        .replace_all(vec![
            make_adapter(first_artifact)
                .registration(host.clone())
                .unwrap(),
        ])
        .unwrap();
    let first_snapshot = compile_snapshot(&first_materialized, &owner);
    let first_active = SessionCapabilityState::new(&first_snapshot)
        .snapshot()
        .unwrap();
    let first = registry
        .acquire_resource(
            &first_snapshot,
            &first_active,
            access(
                &first_snapshot,
                first_active.generation,
                &owner,
                RESOURCE_ID,
            ),
        )
        .await
        .unwrap();
    let JavaScriptHostState::Running {
        generation: first_generation,
        ..
    } = host.state()
    else {
        panic!("first resource acquisition must start the shared Host");
    };
    host.stop_generation(first_generation).await.unwrap();

    let second_materialized = registry
        .replace_all(vec![
            make_adapter(second_artifact)
                .registration(host.clone())
                .unwrap(),
        ])
        .unwrap();
    let second_snapshot = compile_snapshot(&second_materialized, &owner);
    let second_active = SessionCapabilityState::new(&second_snapshot)
        .snapshot()
        .unwrap();
    let second = registry
        .acquire_resource(
            &second_snapshot,
            &second_active,
            access(
                &second_snapshot,
                second_active.generation,
                &owner,
                RESOURCE_ID,
            ),
        )
        .await
        .unwrap();
    assert!(
        !Arc::ptr_eq(&first.handle, &second.handle),
        "a compatible contract cannot reuse a resource handle from an old Artifact"
    );

    registry
        .release_resources(&ScopeKey::from("session:fixture-session"))
        .await
        .unwrap();
    let JavaScriptHostState::Running {
        generation: second_generation,
        ..
    } = host.state()
    else {
        panic!("second resource acquisition must start a new Host generation");
    };
    host.stop_generation(second_generation).await.unwrap();
}
