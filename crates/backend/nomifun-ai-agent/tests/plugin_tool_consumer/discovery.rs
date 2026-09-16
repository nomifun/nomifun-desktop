//! Real Node + Kernel + frozen Role choice + Nomi ToolSearch consumption.
use super::*;
use nomifun_agent_contracts::{
    ArtifactFileDigest, ExactRoleContractRef, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile, JavaScriptEntrypointMetadata,
    MINIMUM_NODE_MAJOR, PLUGIN_N1_SCHEMA_VERSION, PLUGIN_PACKAGE_PROFILE_VERSION,
    PluginPackageArtifactV1, PluginPackageV1Manifest, RoleContractKey, RoleContractManifest,
    RoleMemberContract, RoleMemberRequirement, RoleProviderContribution,
    RoleProviderMemberContribution, RoleProviderSelection,
};
use nomifun_ai_agent::tool_discovery::{self as discovery, CAPABILITY_ID, ROLE_ID};
use nomifun_js_host::{
    ExtensionHostSupervisor, JavaScriptHostConfig, JavaScriptHostLimits, JavaScriptHostState,
};
use nomifun_js_kernel_adapter::{JsKernelPluginAdapter, PluginPackageInput};
use nomifun_js_runtime::{NodeProbeCandidate, NodeRuntimeResolver};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;

const IMPL: &str = "example.discovery";
const BUILTIN: &str = "discovery-builtin";

fn schema_map() -> Arc<SchemaMap> {
    Arc::new(SchemaMap {
        schemas: discovery::schemas(),
    })
}

fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discovery/main.mjs")
}

fn builtin() -> (PluginRegistration, RoleContractManifest) {
    let (base, _) = registration(
        'a',
        "unused",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut metadata = base.metadata;
    let manifest = &mut metadata.manifest.payload;
    let capability = tool_capability(
        &PackageRef {
            id: PACKAGE.into(),
            version: VERSION.into(),
        },
        CAPABILITY_ID,
        vec![CapabilityConsumer::Agent],
        vec![discovery::action()],
    );
    let role = RoleContractManifest {
        key: RoleContractKey {
            role_id: ROLE_ID.into(),
            contract_version: VERSION.into(),
        },
        members: vec![RoleMemberContract {
            capability: CapabilityRef {
                id: CAPABILITY_ID.into(),
                version: VERSION.into(),
            },
            capability_manifest_digest: digest_payload(&capability).unwrap(),
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    manifest.contributions = PackageContributions {
        capabilities: vec![capability],
        role_contracts: vec![role.clone()],
        role_providers: vec![provider(&role, false)],
        ..Default::default()
    };
    metadata.manifest = ArtifactEnvelope::new(metadata.manifest.payload).unwrap();
    metadata.mount_id = BUILTIN.into();
    metadata.source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: BUILTIN.into(),
        source_digest: None,
    };
    metadata.context.source = metadata.source.clone();
    metadata.context.identity.mount_id = BUILTIN.into();
    metadata.context.state.mount_id = BUILTIN.into();
    metadata.registrar.identity.mount_id = BUILTIN.into();
    metadata.registrar.declared_capability_ids = BTreeSet::from([CAPABILITY_ID.into()]);
    metadata.registrar.declared_role_ids = BTreeSet::from([ROLE_ID.into()]);
    metadata
        .registrar
        .allowed_operations
        .insert(PluginRegistrarOperation::ContributeRoleProvider);
    let mut registration = PluginRegistration::new(metadata);
    registration
        .add_role_action_handler(
            ROLE_ID.into(),
            CAPABILITY_ID.into(),
            Arc::new(discovery::BuiltinDiscovery),
        )
        .unwrap();
    (registration, role)
}

fn provider(role: &RoleContractManifest, mapped: bool) -> RoleProviderContribution {
    RoleProviderContribution {
        role: ExactRoleContractRef {
            key: role.key.clone(),
            contract_digest: digest_payload(role).unwrap(),
        },
        display: display("Discovery provider"),
        members: BTreeMap::from([(
            CAPABILITY_ID.into(),
            RoleProviderMemberContribution {
                implementation: mapped.then(|| CapabilityRef {
                    id: IMPL.into(),
                    version: VERSION.into(),
                }),
                supported_platforms: vec![PlatformConstraint::Any],
                required_resource_kinds: BTreeSet::new(),
            },
        )]),
    }
}

fn artifact(role: &RoleContractManifest) -> PluginPackageArtifactV1 {
    let main = std::fs::read(source()).unwrap();
    let hash: DigestHex = hex::encode(Sha256::digest(&main)).into();
    let package = PackageRef {
        id: "example.discovery-package".into(),
        version: VERSION.into(),
    };
    let manifest = PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: VERSION.into(),
            host_contract_version: VERSION.into(),
            package_id: package.id.clone(),
            package_version: package.version.clone(),
            display: display("JS reverse discovery"),
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(json!({"type":"object"})),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: JavaScriptEntrypointMetadata {
                normalized_relative_path: "main.mjs".into(),
                module_digest: hash.clone(),
                host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            }
            .into(),
            contributions: PackageContributions {
                capabilities: vec![tool_capability(
                    &package,
                    IMPL,
                    vec![CapabilityConsumer::Agent],
                    vec![discovery::action()],
                )],
                role_providers: vec![provider(role, true)],
                ..Default::default()
            },
        },
        schemas: discovery::schemas(),
        supported_targets: BTreeSet::from(["x86_64-pc-windows-msvc".into()]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: "b".repeat(64).into(),
        credential_slots: Vec::new(),
    };
    PluginPackageArtifactV1::new(
        "discovery-artifact".into(),
        manifest,
        vec![ArtifactFileDigest {
            normalized_relative_path: "main.mjs".into(),
            digest: hash,
            size_bytes: main.len() as u64,
        }],
    )
    .unwrap()
}

async fn host() -> Arc<ExtensionHostSupervisor> {
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join(if cfg!(windows) { "node.exe" } else { "node" }))
        .find(|path| path.is_file())
        .expect("Node is required for discovery integration")
        .canonicalize()
        .unwrap();
    let probe = NodeRuntimeResolver::default()
        .probe(&NodeProbeCandidate::new(
            nomifun_agent_contracts::NodeRuntimeSourceKind::ProcessPath,
            executable.clone(),
        ))
        .await;
    Arc::new(
        ExtensionHostSupervisor::new(JavaScriptHostConfig {
            node_executable: executable,
            runtime: probe.fingerprint.expect("supported Node runtime"),
            host_module: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../nomifun-js-host/assets/extension-host.mjs")
                .canonicalize()
                .unwrap(),
            limits: JavaScriptHostLimits {
                hello_timeout: Duration::from_secs(5),
                request_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        })
        .unwrap(),
    )
}

fn revision(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    id: &str,
    mount: Option<&str>,
) -> AgentPresetRevision {
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: "0190f5fe-7c00-7a00-8000-000000000004".into(),
            revision: 1,
            revision_digest: "".into(),
        },
        payload: AgentPresetRevisionPayload {
            runtime_engine: None,
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VERSION.into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![selection(id, &[discovery::ACTION_ID])],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: mount
                .map(|mount| {
                    (
                        ROLE_ID.into(),
                        RoleProviderSelection {
                            role: registry
                                .role_provider(&ROLE_ID.into(), &mount.into())
                                .unwrap()
                                .provider
                                .role
                                .clone(),
                            provider_mount_id: mount.into(),
                        },
                    )
                })
                .into_iter()
                .collect(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
        },
        contribution_locks: vec![
            registry
                .capability(&id.into())
                .unwrap()
                .contribution_lock
                .clone(),
        ],
        created_by: OWNER.into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

struct Candidate(&'static str);
#[async_trait]
impl Tool for Candidate {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "shared discovery candidate"
    }
    fn input_schema(&self) -> JsonSchema {
        json!({"type":"object"})
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        true
    }
    fn is_deferred(&self) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    async fn execute(&self, _: Value) -> ToolResult {
        ToolResult::text("candidate")
    }
}

fn make_tools(session: &nomifun_ai_agent::NomiPluginToolSession) -> ToolRegistry {
    let mut allowed = Vec::new();
    let mut deferred = vec!["ToolSearch".into()];
    session.extend_tool_policy(&mut allowed, &mut deferred);
    assert!(allowed.iter().any(|name| name == "ToolSearch"));
    assert!(deferred.is_empty(), "discovery cannot defer itself");
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(
        tools.deferred_state(),
    )));
    for name in ["Alpha", "Beta"] {
        tools.register(Box::new(Candidate(name)));
    }
    session.register_into(&mut tools).unwrap();
    tools
}

async fn search(tools: &ToolRegistry, query: &str) -> ToolResult {
    tools
        .get("ToolSearch")
        .unwrap()
        .execute(json!({"query":query}))
        .await
}

struct ProductDiscoveryInvoker {
    expected: nomifun_agent_contracts::ResolvedCapability,
    calls: AtomicUsize,
}

#[async_trait]
impl NomiPluginProductToolInvoker for ProductDiscoveryInvoker {
    async fn invoke(
        &self,
        request: NomiPluginProductToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        assert_eq!(request.capability(), &self.expected);
        assert_eq!(request.action(), &discovery::action());
        assert_eq!(
            request.operation_id().as_ref(),
            request.idempotency_key().as_ref()
        );
        assert_eq!(
            request.operation_id().as_ref(),
            request.correlation_id().as_ref()
        );
        let input = &request.input().0;
        assert_eq!(input["limit"], 5);
        let candidates = input["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            assert_eq!(candidate.as_object().unwrap().len(), 3);
            assert!(candidate.get("name").is_some());
            assert!(candidate.get("description").is_some());
            assert!(candidate.get("aliases").is_some());
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let output = match input["query"].as_str().unwrap() {
            "outside" => json!({"names":["Beta", "Missing"]}),
            "duplicate" => json!({"names":["Beta", "Beta"]}),
            "extra" => json!({"names":["Beta"], "unexpected":true}),
            "reject" => {
                return Err(NomiPluginToolError::Contract(
                    "private product diagnostic".into(),
                ));
            }
            _ => json!({"names":["Beta", "Alpha"]}),
        };
        Ok(StrictJsonValue(output))
    }
}

#[tokio::test]
async fn product_discovery_uses_frozen_adapter_without_exposing_hidden_action() {
    let (mut capability, _) = plugin_product_fixture();
    capability.actions = vec![discovery::action()];
    let compiled = compile_plugin_product_fixture(&capability);
    let invoker = Arc::new(ProductDiscoveryInvoker {
        expected: compiled.content().contributions().next().unwrap().clone(),
        calls: AtomicUsize::new(0),
    });
    let kernel = Arc::new(
        KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let base = KernelNomiPluginToolSession::materialize(
        kernel,
        Arc::new(compiled.clone()),
        owner(),
        SESSION.into(),
        format!("session:{SESSION}").into(),
        Arc::new(SchemaMap::default()),
    )
    .await
    .unwrap();
    assert!(
        base.register_into(&mut ToolRegistry::new()).is_err(),
        "an unbound Product must not fall back to builtin discovery"
    );
    assert!(
        base.clone()
            .bind_hosted_execution(product_bindings(Vec::new(), invoker.clone()))
            .is_err()
    );
    let actions = KernelNomiPluginToolSession::materialize_plugin_product_actions(
        &compiled,
        &owner(),
        &SESSION.into(),
        &format!("session:{SESSION}").into(),
        Arc::new(PluginProductSchemaMap {
            schemas: discovery::schemas(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(actions.len(), 1);
    let loaded = base
        .bind_hosted_execution(product_bindings(actions, invoker.clone()))
        .unwrap();
    assert!(loaded.actions().is_empty());
    assert!(loaded.plugin_product_actions().is_empty());
    assert_eq!(
        loaded.provider_names_for(capability.capability.id.as_ref(), false),
        vec!["ToolSearch"]
    );
    let tools = make_tools(&loaded);
    assert_eq!(
        tools.tool_names().len(),
        3,
        "only ToolSearch and two candidates"
    );
    let result = search(&tools, "shared").await;
    assert!(!result.is_error, "{}", result.content);
    let matches: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(matches[0]["name"], "Beta");
    for query in ["outside", "duplicate", "extra", "reject"] {
        let tools = make_tools(&loaded);
        let result = search(&tools, query).await;
        assert!(result.is_error, "{query}: {}", result.content);
        assert!(!result.content.contains("private product diagnostic"));
        assert!(tools.activated_deferred_tool_identities().is_empty());
    }
    assert_eq!(invoker.calls.load(Ordering::SeqCst), 5);

    let mut schemas = discovery::schemas();
    schemas.insert(
        discovery::action().output_schema,
        StrictJsonValue(json!({"type":"object"})),
    );
    assert!(
        KernelNomiPluginToolSession::materialize_plugin_product_actions(
            &compiled,
            &owner(),
            &SESSION.into(),
            &format!("session:{SESSION}").into(),
            Arc::new(PluginProductSchemaMap { schemas }),
        )
        .await
        .is_err(),
        "discovery output schema must match the frozen digest"
    );
}

#[tokio::test]
async fn frozen_provider_choice_reaches_real_js_without_exposing_a_second_toolsearch() {
    let host = host().await;
    let (builtin, role) = builtin();
    let artifact = artifact(&role);
    let temp = tempfile::TempDir::new().unwrap();
    let adapter = JsKernelPluginAdapter::new(PluginPackageInput {
        config: ValidatedPluginConfig {
            schema_digest: digest_payload(&artifact.manifest.payload.package.config_schema)
                .unwrap(),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        artifact,
        mount_id: MOUNT.into(),
        package_root: source()
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf(),
        credential_bindings: Vec::new(),
        data_dir: temp.path().to_path_buf(),
    })
    .unwrap();
    let kernel = Arc::new(
        KernelRegistry::new(
            MaterializationPolicy {
                host_contract_version: VERSION.into(),
                available_runtime_features: BTreeSet::new(),
                allowed_sources: BTreeSet::from([
                    PluginSourceKind::Bundled,
                    PluginSourceKind::ManagedLocal,
                ]),
            },
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let materialized = kernel
        .replace_all(vec![builtin, adapter.registration(host.clone()).unwrap()])
        .unwrap();
    for (id, mount, expected) in [
        (CAPABILITY_ID, Some(BUILTIN), "Alpha"),
        (CAPABILITY_ID, Some(MOUNT), "Beta"),
        (IMPL, None, "Beta"),
    ] {
        let compiled = compile_revision(&materialized, revision(&materialized, id, mount));
        let loaded = session(kernel.clone(), compiled, schema_map()).await;
        assert!(
            loaded.actions().is_empty(),
            "hidden strategy must not become an LLM action"
        );
        assert_eq!(loaded.provider_names_for(id, false), vec!["ToolSearch"]);
        let tools = make_tools(&loaded);
        let result = search(&tools, "shared").await;
        assert!(!result.is_error, "{}", result.content);
        let matches: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(matches[0]["name"], expected);
        assert!(matches[0].get("parameters").is_none());
        assert_eq!(
            tools
                .tool_names()
                .iter()
                .filter(|name| **name == "ToolSearch")
                .count(),
            1
        );
    }
    let compiled = compile_revision(
        &materialized,
        revision(&materialized, CAPABILITY_ID, Some(MOUNT)),
    );
    let loaded = session(kernel.clone(), compiled, schema_map()).await;
    for query in ["outside", "duplicate", "reject"] {
        let tools = make_tools(&loaded);
        let result = search(&tools, query).await;
        assert!(result.is_error, "{query}: {}", result.content);
        assert!(!result.content.contains("private plugin diagnostic"));
        assert!(tools.activated_deferred_tool_identities().is_empty());
    }
    let tools = make_tools(&loaded);
    assert!(
        !search(&tools, "a").await.is_error,
        "custom policies own matching heuristics"
    );
    let tools = make_tools(&loaded);
    assert!(search(&tools, &"x".repeat(4097)).await.is_error);
    assert!(tools.activated_deferred_tool_identities().is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), search(&tools, "wait"))
            .await
            .is_err()
    );
    assert!(
        tools.activated_deferred_tool_identities().is_empty(),
        "cancel must not commit a late result"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let result = search(&tools, "cancelled").await;
        assert!(!result.is_error, "{}", result.content);
        if result.content.contains("Beta") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cancellation did not reach JS AbortSignal"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let tools = make_tools(&loaded);
    let timeout = search(&tools, "wait").await;
    assert!(timeout.is_error && timeout.content.contains("timed out"));
    assert!(tools.activated_deferred_tool_identities().is_empty());
    kernel.replace_all(Vec::new()).unwrap();
    let tools = make_tools(&loaded);
    assert!(
        search(&tools, "shared").await.is_error,
        "withdrawal must not fall back to builtin"
    );
    assert!(tools.activated_deferred_tool_identities().is_empty());
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        // Timeout drops the invocation immediately; JS cancellation and the
        // Host's pending-request cleanup complete asynchronously. Wait for the
        // existing quiescent fence rather than forcing the process to stop.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            match host.stop_generation(generation).await {
                Ok(_) => break,
                Err(nomifun_js_host::JavaScriptHostError::NotQuiescent { .. }) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "timed-out discovery retained a Host invocation"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("Host cleanup failed: {error}"),
            }
        }
    }
}
