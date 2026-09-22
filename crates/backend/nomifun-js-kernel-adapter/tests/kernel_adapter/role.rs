use super::*;
use async_trait::async_trait;
use nomifun_agent_contracts::{
    ArtifactEnvelope, ExactRoleContractRef, InProcessEntrypointMetadata, InstallationRoleBinding,
    PluginRegistrarOperation, PluginSourceMetadata, RoleContractKey, RoleContractManifest,
    RoleMemberContract, RoleMemberRequirement, RoleProviderContribution,
    RoleProviderMemberContribution, RoleProviderSelection,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, CompiledSnapshot, ContextContributionFactory,
    ContextContributionRequest, ContextContributionResult, KernelError, MaterializedRegistry,
    PluginRegistration, ResourceHandle, ResourceHandleIdentity, ResourceProviderFactory,
    ResourceProviderRequest, ResourceProviderResult, RoleMemberAdmission,
    RoleMemberInvocationRequest, RoleToolOperationRequest, resolve_exact_role_provider_lock,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "requirements.rs"]
mod requirements;
#[path = "dependencies.rs"]
mod dependencies;

const ROLE: &str = "system.fixture";
const BUILTIN_MOUNT: &str = "fixture-builtin";
const USER_MOUNT: &str = "fixture-user";
const MEMBERS: [&str; 3] = [
    "platform.fixture.tool",
    "platform.fixture.context",
    "platform.fixture.resource",
];
const IMPLEMENTATIONS: [&str; 3] = [TOOL_ID, CONTEXT_ID, RESOURCE_ID];

fn contract(base: &PluginPackageArtifactV1) -> (RoleContractManifest, Vec<CapabilityManifest>) {
    let capabilities = IMPLEMENTATIONS
        .iter()
        .zip(MEMBERS)
        .map(|(id, member)| {
            let mut value = base
                .manifest
                .payload
                .package
                .contributions
                .capabilities
                .iter()
                .find(|value| value.id.as_ref() == *id)
                .unwrap()
                .clone();
            value.id = member.into();
            value.contribution_id = format!("capability:{member}").into();
            value.package.id = "platform.fixture".into();
            // The public contract does not copy an implementation's private
            // capability dependencies. Provider fixtures may differ freely.
            value.requires.clear();
            value
        })
        .collect::<Vec<_>>();
    let contract = RoleContractManifest {
        key: RoleContractKey {
            role_id: ROLE.into(),
            contract_version: VERSION.into(),
        },
        members: capabilities
            .iter()
            .map(|capability| RoleMemberContract {
                capability: CapabilityRef {
                    id: capability.id.clone(),
                },
                capability_manifest_digest: digest_payload(capability).unwrap(),
                requirement: RoleMemberRequirement::Required,
            })
            .collect(),
        serialized_target_resource_kind: None,
    };
    (contract, capabilities)
}

fn provider(contract: &RoleContractManifest, mapped: bool) -> RoleProviderContribution {
    RoleProviderContribution {
        role: ExactRoleContractRef {
            key: contract.key.clone(),
            contract_digest: digest_payload(contract).unwrap(),
        },
        display: display(
            "Fixture Provider",
            "Independent implementations of the same Role",
        ),
        members: MEMBERS
            .into_iter()
            .zip(IMPLEMENTATIONS)
            .map(|(member, implementation)| {
                (
                    member.into(),
                    RoleProviderMemberContribution {
                        implementation: mapped.then(|| CapabilityRef {
                            id: implementation.into(),
                        }),
                        supported_platforms: vec![PlatformConstraint::Any],
                        required_resource_kinds: if implementation == RESOURCE_ID {
                            BTreeSet::from([RESOURCE_KIND.into()])
                        } else {
                            BTreeSet::new()
                        },
                    },
                )
            })
            .collect(),
    }
}

fn mapped_artifact(main: &[u8]) -> PluginPackageArtifactV1 {
    let base = artifact(main);
    let (contract, _) = contract(&base);
    let mut manifest = base.manifest.payload.clone();
    manifest.package.contributions.role_providers = vec![provider(&contract, true)];
    PluginPackageArtifactV1::new(base.artifact_id, manifest, base.files).unwrap()
}

#[tokio::test]
async fn provider_cannot_change_the_facade_context_phase() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let original = mapped_artifact(&main);
    let temp = TempDir::new().unwrap();
    let original_adapter = adapter(original.clone(), &temp);
    let host = host().await;
    let builtin = builtin(&original_adapter, host.clone(), Arc::new(AtomicUsize::new(0)));
    let mut manifest = original.manifest.payload;
    manifest.package.contributions.capabilities.iter_mut()
        .find(|value| value.id.as_ref() == CONTEXT_ID).unwrap().contributions.context_phase =
            nomifun_agent_contracts::ContextContributionPhase::BeforeTurn;
    let incompatible = PluginPackageArtifactV1::new(original.artifact_id, manifest, original.files).unwrap();
    let candidate = adapter(incompatible, &temp);
    assert!(registry().replace_all(vec![builtin, candidate.registration(host.clone()).unwrap()]).is_err());
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn dynamic_context_input_reaches_direct_and_selected_role_javascript() {
    use nomifun_agent_contracts::{ContextContributionInput, ContextContributionPhase, ContextTurnInput};
    for mapped in [false, true] {
        let main = std::fs::read(fixture("main.mjs")).unwrap();
        let base = artifact(&main);
        let mut manifest = base.manifest.payload.clone();
        manifest.package.contributions.capabilities.iter_mut()
            .find(|value| value.id.as_ref() == CONTEXT_ID).unwrap()
            .contributions.context_phase = ContextContributionPhase::BeforeTurn;
        let dynamic = PluginPackageArtifactV1::new(base.artifact_id, manifest, base.files).unwrap();
        let mut manifest = dynamic.manifest.payload.clone();
        if mapped {
            manifest.package.contributions.role_providers = vec![provider(&contract(&dynamic).0, true)];
        }
        let artifact = PluginPackageArtifactV1::new(dynamic.artifact_id, manifest, dynamic.files).unwrap();
        let temp = TempDir::new().unwrap();
        let adapter = adapter(artifact, &temp);
        let host = host().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registrations = vec![adapter.registration(host.clone()).unwrap()];
        if mapped { registrations.push(builtin(&adapter, host.clone(), calls.clone())); }
        let registry = registry();
        let materialized = registry.replace_all(registrations).unwrap();
        let owner = PrincipalRef { principal_kind: "user".into(), principal_id: "fixture-owner".into() };
        let snapshot = if mapped { compile(&materialized, &owner, true) } else { compile_snapshot(&materialized, &owner) };
        let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
        let id = if mapped { MEMBERS[1] } else { CONTEXT_ID };
        assert!(registry.contribute_context(&snapshot, &active, access(&snapshot, active.generation, &owner, id)).await.is_err());
        assert_eq!(host.process_count(), 0, "wrong phase must not start a JS process");
        for text in ["first", "second"] {
            let input = ContextContributionInput::BeforeTurn { turn: ContextTurnInput {
                source_message_id: format!("source-{text}"), text: text.into(), image_media_types: vec!["image/png".into()], cs_dialogue_id: None,
            }};
            let result = registry.contribute_context_with_input(&snapshot, &active,
                access(&snapshot, active.generation, &owner, id), input.clone()).await.unwrap().value.unwrap();
            assert_eq!(result.0["input"], serde_json::to_value(input).unwrap());
            assert_eq!(result.0["contributionId"], "fixture.context.contribution");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0, "selected JS Context never falls back to builtin");
        if !mapped {
            let pending = registry.contribute_context_with_input(&snapshot, &active,
                access(&snapshot, active.generation, &owner, id), ContextContributionInput::BeforeTurn {
                    turn: ContextTurnInput { source_message_id: "pending".into(), text: "wait-for-cancel".into(), image_media_types: vec![], cs_dialogue_id: None },
                });
            assert!(tokio::time::timeout(Duration::from_millis(100), pending).await.is_err());
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            loop {
                let mut request = invoke_request(&snapshot, &owner);
                request.capability_id = TOOL_ID.into();
                request.action_id = RELEASE_COUNT_ACTION.into();
                let result = registry.invoke(&snapshot, &active, request).await.unwrap();
                if result.0["contextCancelCount"] == 1 { break; }
                assert!(tokio::time::Instant::now() < deadline, "dropped turn Context did not reach the JS AbortSignal");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        registry.replace_all(Vec::new()).unwrap();
        assert!(registry.contribute_context_with_input(&snapshot, &active,
            access(&snapshot, active.generation, &owner, id), ContextContributionInput::BeforeTurn {
                turn: ContextTurnInput { source_message_id: "gone".into(), text: "gone".into(), image_media_types: vec![], cs_dialogue_id: None },
            }).await.is_err());
        if let JavaScriptHostState::Running { generation, .. } = host.state() {
            host.stop_generation(generation).await.unwrap();
        }
    }
}

fn adapter(artifact: PluginPackageArtifactV1, temp: &TempDir) -> JsKernelPluginAdapter {
    JsKernelPluginAdapter::new(PluginPackageInput {
        config: ValidatedPluginConfig {
            schema_digest: digest_payload(&artifact.manifest.payload.package.config_schema)
                .unwrap(),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        artifact,
        mount_id: USER_MOUNT.into(),
        package_root: fixture("main.mjs")
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf(),
        credential_bindings: Vec::new(),
        data_dir: temp.path().to_path_buf(),
    })
    .unwrap()
}

fn builtin(
    adapter: &JsKernelPluginAdapter,
    host: Arc<ExtensionHostSupervisor>,
    calls: Arc<AtomicUsize>,
) -> PluginRegistration {
    let (contract, capabilities) = contract(adapter.artifact());
    let mut metadata = adapter.registration(host).unwrap().metadata;
    let mut manifest = metadata.manifest.payload;
    manifest.package_id = "platform.fixture".into();
    manifest.entrypoint = InProcessEntrypointMetadata {
        entrypoint_profile: "trusted-in-process".into(),
        entrypoint_id: "platform.fixture".into(),
        contract_version: VERSION.into(),
    }
    .into();
    manifest.contributions = PackageContributions {
        capabilities,
        role_providers: vec![provider(&contract, false)],
        role_contracts: vec![contract],
        ..Default::default()
    };
    metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    metadata.mount_id = BUILTIN_MOUNT.into();
    metadata.source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: BUILTIN_MOUNT.into(),
        source_digest: None,
    };
    metadata.context.identity.package.id = "platform.fixture".into();
    metadata.context.identity.mount_id = BUILTIN_MOUNT.into();
    metadata.context.source = metadata.source.clone();
    metadata.context.state.package_id = "platform.fixture".into();
    metadata.context.state.mount_id = BUILTIN_MOUNT.into();
    metadata.context.cancellation.scope_key = format!("mount:{BUILTIN_MOUNT}").into();
    metadata.context.managed_task_registration.scope_key = format!("mount:{BUILTIN_MOUNT}").into();
    metadata.registrar.identity = metadata.context.identity.clone();
    metadata.registrar.declared_capability_ids =
        MEMBERS.into_iter().map(CapabilityId::from).collect();
    metadata.registrar.declared_role_ids = BTreeSet::from([ROLE.into()]);
    metadata
        .registrar
        .allowed_operations
        .insert(PluginRegistrarOperation::ContributeRoleProvider);
    let mut registration = PluginRegistration::new(metadata);
    let export = Arc::new(BuiltinExport(calls));
    registration
        .add_role_action_handler(ROLE.into(), MEMBERS[0].into(), export.clone())
        .unwrap();
    registration
        .add_role_context_factory(ROLE.into(), MEMBERS[1].into(), export.clone())
        .unwrap();
    registration
        .add_role_resource_factory(ROLE.into(), MEMBERS[2].into(), export)
        .unwrap();
    registration
}

struct BuiltinExport(Arc<AtomicUsize>);
#[async_trait]
impl CapabilityHandler for BuiltinExport {
    async fn invoke(
        &self,
        _: CapabilityInvocationContext,
        _: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(StrictJsonValue(json!({"builtin": true})))
    }
}
#[async_trait]
impl ContextContributionFactory for BuiltinExport {
    async fn contribute(
        &self,
        _: ContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ContextContributionResult {
            value: Some(StrictJsonValue(json!({"builtin": true}))),
        })
    }
}
struct BuiltinResource(ResourceHandleIdentity);
#[async_trait]
impl ResourceHandle for BuiltinResource {
    fn identity(&self) -> &ResourceHandleIdentity {
        &self.0
    }
    async fn release(&self) -> Result<(), KernelError> {
        Ok(())
    }
}
#[async_trait]
impl ResourceProviderFactory for BuiltinExport {
    async fn acquire(
        &self,
        request: ResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let binding = &request.context.resource_bindings[0];
        Ok(ResourceProviderResult {
            handle: Arc::new(BuiltinResource(ResourceHandleIdentity {
                binding_id: binding.binding_id.clone(),
                resource_kind: binding.resource_kind.clone(),
                resource_id: binding.resource_id.clone(),
            })),
        })
    }
}

fn registry() -> KernelRegistry {
    KernelRegistry::new(
        MaterializationPolicy {
            host_contract_version: VERSION.into(),
            available_runtime_features: BTreeSet::new(),
            allowed_sources: BTreeSet::from([
                PluginSourceKind::ManagedLocal,
                PluginSourceKind::Bundled,
            ]),
        },
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap()
}

fn selection(materialized: &MaterializedRegistry, mount: &str) -> RoleProviderSelection {
    RoleProviderSelection {
        role: materialized
            .role_provider(&ROLE.into(), &mount.into())
            .unwrap()
            .provider
            .role
            .clone(),
        provider_mount_id: mount.into(),
    }
}

fn compile(
    materialized: &MaterializedRegistry,
    owner: &PrincipalRef,
    use_override: bool,
) -> CompiledSnapshot {
    let mut revision = revision(owner, materialized);
    for (selected, id) in revision
        .payload
        .enabled_capabilities
        .iter_mut()
        .zip(MEMBERS[..2].iter().copied())
    {
        selected.capability.id = id.into();
    }
    revision.contribution_locks = MEMBERS[..2]
        .iter()
        .copied()
        .map(|id| {
            materialized
                .capability(&id.into())
                .unwrap()
                .contribution_lock
                .clone()
        })
        .collect();
    if use_override {
        revision
            .payload
            .system_role_provider_overrides
            .insert(ROLE.into(), selection(materialized, USER_MOUNT));
    }
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let mut env = environment(materialized.registry_digest.clone());
    env.installation_role_bindings.insert(
        ROLE.into(),
        InstallationRoleBinding {
            selection: selection(materialized, BUILTIN_MOUNT),
            binding_version: 1,
            updated_at_ms: 1,
        },
    );
    AgentPresetCompiler::compile(
        materialized,
        &env,
        CompileRequest {
            revision,
            plugin_product_capabilities: Vec::new(),
            principal: owner.clone(),
            scene: "fixture".into(),
            surface: "desktop".into(),
            audience: "test".into(),
            created_at_ms: 2,
            resolver_run_id: "role-compile".into(),
        },
    )
    .unwrap()
    .with_target_resource_bindings(
        owner,
        vec![TypedResourceBinding {
            binding_id: "fixture-binding".into(),
            resource_kind: RESOURCE_KIND.into(),
            resource_id: "fixture-resource".into(),
            owner_id: owner.principal_id.clone(),
            operations: BTreeSet::from(["acquire".into()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        }],
    )
    .unwrap()
}

fn invoke_request(
    snapshot: &CompiledSnapshot,
    owner: &PrincipalRef,
) -> CapabilityInvocationRequest {
    CapabilityInvocationRequest {
        principal: owner.clone(),
        session_owner: owner.clone(),
        agent_session_id: "fixture-session".into(),
        turn_id: "fixture-turn".into(),
        operation_id: "role-tool-operation".into(),
        idempotency_key: "role-tool-key".into(),
        correlation_id: "role-tool-correlation".into(),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: SessionCapabilityState::new(snapshot)
            .snapshot()
            .unwrap()
            .generation,
        capability_id: MEMBERS[0].into(),
        action_id: TOOL_ACTION.into(),
        resource_binding_ids: BTreeSet::new(),
        state_scope_key: "session:fixture-session".into(),
        input: StrictJsonValue(json!({"value": 17})),
    }
}

#[tokio::test]
async fn user_provider_replaces_builtin_tool_context_resource_without_identity_override() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let adapter = adapter(mapped_artifact(&main), &temp);
    let host = host().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let builtin = builtin(&adapter, host.clone(), calls.clone());
    let registry = registry();
    let materialized = registry
        .replace_all(vec![
            builtin.clone(),
            adapter.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let default = compile(&materialized, &owner, false);
    let default_active = SessionCapabilityState::new(&default).snapshot().unwrap();
    assert_eq!(
        registry
            .invoke(&default, &default_active, invoke_request(&default, &owner))
            .await
            .unwrap()
            .0["builtin"],
        true
    );
    assert_eq!(
        host.process_count(),
        0,
        "unselected JS implementation is not started"
    );
    let snapshot = compile(&materialized, &owner, true);
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    assert!(
        snapshot.resolved_capability(&TOOL_ID.into()).is_none(),
        "mapping is not an extra model Tool"
    );
    let result = registry
        .invoke(&snapshot, &active, invoke_request(&snapshot, &owner))
        .await
        .unwrap();
    assert_eq!(result.0["contributionId"], "fixture.tool.contribution");
    assert_eq!(result.0["input"]["value"], 17);
    assert!(result.0["resourceBindings"].as_array().unwrap().is_empty());
    let context = registry
        .contribute_context(
            &snapshot,
            &active,
            access(&snapshot, active.generation, &owner, MEMBERS[1]),
        )
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(context.0["contributionId"], "fixture.context.contribution");
    assert!(context.0["resourceBindings"].as_array().unwrap().is_empty());
    registry
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();
    let mut count = invoke_request(&snapshot, &owner);
    count.action_id = RELEASE_COUNT_ACTION.into();
    assert_eq!(
        registry.invoke(&snapshot, &active, count).await.unwrap().0["releaseCount"],
        0
    );

    // Non-Agent callers use their existing operation admission, no synthetic Session.
    let lock = resolve_exact_role_provider_lock(
        &materialized,
        &ROLE.into(),
        &selection(&materialized, USER_MOUNT),
        &BTreeSet::from([MEMBERS[0].into()]),
        &BTreeMap::new(),
        &environment(materialized.registry_digest.clone()),
    )
    .unwrap();
    let operation = registry
        .invoke_role_tool(RoleToolOperationRequest {
            member: RoleMemberInvocationRequest {
                principal: owner.clone(),
                session_owner: owner.clone(),
                turn_id: None,
                operation_id: "role-operation".into(),
                correlation_id: "role-correlation".into(),
                capability_id: MEMBERS[0].into(),
                resource_binding_ids: BTreeSet::new(),
                state_scope_key: "operation:fixture".into(),
                admission: RoleMemberAdmission::Operation {
                    provider_lock: lock,
                    registry_generation: materialized.generation,
                    registry_digest: materialized.registry_digest.clone(),
                    resource_bindings: Vec::new(),
                },
            },
            action_id: TOOL_ACTION.into(),
            idempotency_key: "operation-key".into(),
            input: StrictJsonValue(json!({})),
        })
        .await
        .unwrap();
    assert_eq!(operation.0["contributionId"], "fixture.tool.contribution");
    // The independently published implementation remains directly usable.
    let direct = compile_snapshot(&materialized, &owner);
    let direct_active = SessionCapabilityState::new(&direct).snapshot().unwrap();
    let mut request = invoke_request(&direct, &owner);
    request.capability_id = TOOL_ID.into();
    assert_eq!(
        registry
            .invoke(&direct, &direct_active, request)
            .await
            .unwrap()
            .0["contributionId"],
        "fixture.tool.contribution"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "selected JS paths never invoke builtin"
    );
    assert_eq!(
        registry
            .invoke(&default, &default_active, invoke_request(&default, &owner))
            .await
            .unwrap()
            .0["builtin"],
        true,
        "new override does not change an existing snapshot"
    );

    registry.replace_all(vec![builtin]).unwrap();
    assert!(
        registry
            .invoke(&snapshot, &active, invoke_request(&snapshot, &owner))
            .await
            .is_err(),
        "withdrawn Provider cannot fall back"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        host.stop_generation(generation).await.unwrap();
    }
}

#[tokio::test]
async fn mapped_provider_checks_implementation_platform_and_exact_artifact() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let adapter = adapter(original.clone(), &temp);
    let host = host().await;
    let builtin = builtin(&adapter, host.clone(), Arc::new(AtomicUsize::new(0)));
    let registry = registry();
    let materialized = registry
        .replace_all(vec![
            builtin.clone(),
            adapter.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let snapshot = compile(&materialized, &owner, true);
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    let mut changed = original.manifest.payload.clone();
    changed
        .package
        .contributions
        .capabilities
        .iter_mut()
        .find(|value| value.id.as_ref() == TOOL_ID)
        .unwrap()
        .supported_platforms = vec![PlatformConstraint::Targets {
        host_targets: BTreeSet::from(["aarch64-apple-darwin".into()]),
        host_surfaces: BTreeSet::new(),
    }];
    let changed = PluginPackageArtifactV1::new(
        original.artifact_id.clone(),
        changed,
        original.files.clone(),
    )
    .unwrap();
    let changed_adapter = self::adapter(changed, &temp);
    let materialized = registry
        .replace_all(vec![
            builtin,
            changed_adapter.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let error = resolve_exact_role_provider_lock(
        &materialized,
        &ROLE.into(),
        &selection(&materialized, USER_MOUNT),
        &BTreeSet::from([MEMBERS[0].into()]),
        &BTreeMap::new(),
        &environment(materialized.registry_digest.clone()),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        KernelError::CapabilityUnavailableOnPlatform { .. }
    ));
    assert!(
        registry
            .invoke(&snapshot, &active, invoke_request(&snapshot, &owner))
            .await
            .is_err(),
        "old artifact lock must not execute changed Package"
    );
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn mapped_provider_rejects_incompatible_callable_contract() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let base_adapter = adapter(original.clone(), &temp);
    let host = host().await;
    let builtin = builtin(&base_adapter, host.clone(), Arc::new(AtomicUsize::new(0)));
    // Private dependency differences are valid and exercised by the real JS
    // dependency tests. Public schemas/effects and declared resource needs
    // must still match the selected member contract.
    for variant in ["schema", "effect", "resource"] {
        let mut manifest = original.manifest.payload.clone();
        let implementation = manifest
            .package
            .contributions
            .capabilities
            .iter_mut()
            .find(|value| value.id.as_ref() == TOOL_ID)
            .unwrap();
        match variant {
            "schema" => {
                implementation.contributions.actions[0].input_schema =
                    fixture_schema_ref("tool-output")
            }
            "effect" => {
                implementation.contributions.actions[0].effect_class = EffectClass::ExternalTransmit
            }
            "resource" => {
                implementation
                    .contributions
                    .resource_kinds
                    .insert(RESOURCE_KIND.into());
            }
            _ => unreachable!(),
        }
        let changed = PluginPackageArtifactV1::new(
            original.artifact_id.clone(),
            manifest,
            original.files.clone(),
        )
        .unwrap();
        let adapter = adapter(changed, &temp);
        let error = registry()
            .replace_all(vec![
                builtin.clone(),
                adapter.registration(host.clone()).unwrap(),
            ])
            .unwrap_err();
        assert!(
            matches!(error, KernelError::InvalidRoleProvider { .. }),
            "{variant}: {error}"
        );
    }
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn user_defined_role_registers_only_typed_facade_exports() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let original = artifact(&main);
    let mut manifest = original.manifest.payload.clone();
    let implementation = manifest.package.contributions.capabilities[0].clone();
    let mut facade = implementation.clone();
    facade.id = "fixture.javascript.facade".into();
    facade.contribution_id = "fixture.javascript.facade.contribution".into();
    let contract = RoleContractManifest {
        key: RoleContractKey {
            role_id: "fixture.javascript.custom".into(),
            contract_version: VERSION.into(),
        },
        members: vec![RoleMemberContract {
            capability: CapabilityRef {
                id: facade.id.clone(),
            },
            capability_manifest_digest: digest_payload(&facade).unwrap(),
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    manifest
        .package
        .contributions
        .role_providers
        .push(RoleProviderContribution {
            role: ExactRoleContractRef {
                key: contract.key.clone(),
                contract_digest: digest_payload(&contract).unwrap(),
            },
            display: display("User Role", "User-defined callable contract"),
            members: BTreeMap::from([(
                facade.id.clone(),
                RoleProviderMemberContribution {
                    implementation: Some(CapabilityRef {
                        id: implementation.id.clone(),
                    }),
                    supported_platforms: vec![PlatformConstraint::Any],
                    required_resource_kinds: BTreeSet::new(),
                },
            )]),
        });
    manifest.package.contributions.role_contracts.push(contract);
    manifest
        .package
        .contributions
        .capabilities
        .push(facade.clone());
    let artifact =
        PluginPackageArtifactV1::new(original.artifact_id, manifest, original.files).unwrap();
    let temp = TempDir::new().unwrap();
    let adapter = adapter(artifact, &temp);
    let host = host().await;
    let registration = adapter.registration(host.clone()).unwrap();
    assert!(!registration.handler_ids().contains(&facade.id));
    assert!(registration.handler_ids().contains(&implementation.id));
    let materialized = registry().replace_all(vec![registration]).unwrap();
    assert!(
        materialized
            .role_contract(&"fixture.javascript.custom".into())
            .is_some()
    );
    assert_eq!(host.process_count(), 0);
}
