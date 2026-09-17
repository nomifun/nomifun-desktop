use super::*;
use nomifun_agent_contracts::RuntimeFeatureRef;

#[path = "conflicts.rs"]
mod conflicts;

const TOOL_FEATURE: &str = "fixture.local-index";
const RESOURCE_FEATURE: &str = "fixture.index-driver";
const UNUSED_FEATURE: &str = "fixture.unused-context";

fn with_requirements(
    base: &PluginPackageArtifactV1,
    context_uses_resource: bool,
) -> PluginPackageArtifactV1 {
    let mut manifest = base.manifest.payload.clone();
    for capability in &mut manifest.package.contributions.capabilities {
        let feature = match capability.id.as_ref() {
            TOOL_ID => TOOL_FEATURE,
            RESOURCE_ID => RESOURCE_FEATURE,
            CONTEXT_ID => UNUSED_FEATURE,
            _ => continue,
        };
        capability
            .requires_runtime_features
            .push(RuntimeFeatureRef {
                id: feature.into(),
                version: VERSION.into(),
            });
        if capability.id.as_ref() == TOOL_ID
            || (context_uses_resource && capability.id.as_ref() == CONTEXT_ID)
        {
            capability
                .contributions
                .resource_kinds
                .insert(RESOURCE_KIND.into());
        }
    }
    let provider = &mut manifest.package.contributions.role_providers[0];
    provider
        .members
        .get_mut(&MEMBERS[0].into())
        .unwrap()
        .required_resource_kinds
        .insert(RESOURCE_KIND.into());
    if context_uses_resource {
        provider
            .members
            .get_mut(&MEMBERS[1].into())
            .unwrap()
            .required_resource_kinds
            .insert(RESOURCE_KIND.into());
    }
    PluginPackageArtifactV1::new(base.artifact_id.clone(), manifest, base.files.clone()).unwrap()
}

fn requirements_registry() -> KernelRegistry {
    KernelRegistry::new(
        MaterializationPolicy {
            host_contract_version: VERSION.into(),
            available_runtime_features: [TOOL_FEATURE, RESOURCE_FEATURE, UNUSED_FEATURE]
                .into_iter()
                .map(Into::into)
                .collect(),
            allowed_sources: BTreeSet::from([
                PluginSourceKind::ManagedLocal,
                PluginSourceKind::Bundled,
            ]),
        },
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap()
}

fn revision_for_members(
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    mount: &str,
    members: &[&str],
) -> AgentPresetRevision {
    let mut revision = revision(owner, registry);
    revision.payload.enabled_capabilities = members
        .iter()
        .map(|id| CapabilitySelection {
            capability: CapabilityRef {
                id: (*id).into(),
                version: VERSION.into(),
            },
            action_allowlist: if *id == MEMBERS[0] {
                BTreeSet::from([TOOL_ACTION.into()])
            } else {
                BTreeSet::new()
            },
        })
        .collect();
    revision.contribution_locks = members
        .iter()
        .map(|id| {
            registry
                .capability(&(*id).into())
                .unwrap()
                .contribution_lock
                .clone()
        })
        .collect();
    revision
        .payload
        .system_role_provider_overrides
        .insert(ROLE.into(), selection(registry, mount));
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

fn compile_members(
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    mount: &str,
    members: &[&str],
    features: &[&str],
) -> Result<CompiledSnapshot, KernelError> {
    let mut env = environment(registry.registry_digest.clone());
    env.available_runtime_features = features.iter().map(|id| (*id).into()).collect();
    AgentPresetCompiler::compile(
        registry,
        &env,
        CompileRequest {
            revision: revision_for_members(registry, owner, mount, members),
            plugin_product_capabilities: Vec::new(),
            principal: owner.clone(),
            scene: "fixture".into(),
            surface: "desktop".into(),
            audience: "test".into(),
            created_at_ms: 3,
            resolver_run_id: "requirements-compile".into(),
        },
    )
}

#[tokio::test]
async fn identical_provider_lock_does_not_reuse_stale_implementation_requirements() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let artifact = mapped_artifact(&main);
    let baseline = adapter(artifact.clone(), &temp);
    let implementation = adapter(with_requirements(&artifact, false), &temp);
    let host = host().await;
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0))),
            implementation.registration(host).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let revision = revision_for_members(&registry, &owner, USER_MOUNT, &[MEMBERS[0]]);
    let mut env = environment(registry.registry_digest.clone());
    env.available_runtime_features = [TOOL_FEATURE, RESOURCE_FEATURE]
        .into_iter()
        .map(Into::into)
        .collect();
    let compiled = compile_members(
        &registry,
        &owner,
        USER_MOUNT,
        &[MEMBERS[0]],
        &[TOOL_FEATURE, RESOURCE_FEATURE],
    )
    .unwrap();
    assert!(AgentPresetCompiler::role_providers_unchanged(
        &registry,
        &env,
        &revision,
        &compiled.envelope,
    ));
    for field in ["resources", "features"] {
        // Model a saved projection from an earlier compiler without changing
        // either the selected Provider lock or capability provenance.
        let mut stale = compiled.envelope.clone();
        let record = &mut stale.content.enabled_capabilities[0];
        if field == "resources" {
            record.required_resource_kinds.clear();
        } else {
            record.required_runtime_features.clear();
        }
        stale.snapshot_ref.snapshot_digest = digest_payload(&stale.content).unwrap();
        stale.validate().unwrap();
        assert_eq!(
            stale.content.resolved_role_providers,
            compiled.content().resolved_role_providers
        );
        assert!(
            !AgentPresetCompiler::role_providers_unchanged(&registry, &env, &revision, &stale,),
            "must refresh stale {field} even though the exact Provider is unchanged"
        );
    }
}

fn binding(owner: &PrincipalRef) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: "fixture-index-binding".into(),
        resource_kind: RESOURCE_KIND.into(),
        resource_id: "fixture-index".into(),
        owner_id: owner.principal_id.clone(),
        operations: BTreeSet::from(["acquire".into()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([("index".into(), "user-local-index".into())]),
    }
}

#[tokio::test]
async fn implicit_resource_factory_dependencies_are_locked_but_not_public_contributions() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let base = mapped_artifact(&main);
    let baseline = adapter(base.clone(), &temp);
    let mut artifact = with_requirements(&base, false);
    let mut manifest = artifact.manifest.payload.clone();
    manifest
        .package
        .contributions
        .capabilities
        .iter_mut()
        .find(|c| c.id.as_ref() == RESOURCE_ID)
        .unwrap()
        .requires = vec![CapabilityRef {
        id: CONTEXT_ID.into(),
        version: VERSION.into(),
    }];
    artifact =
        PluginPackageArtifactV1::new(artifact.artifact_id, manifest, artifact.files).unwrap();
    let implementation = adapter(artifact, &temp);
    let host = host().await;
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0))),
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    // An unconsumed Context member was previously irrelevant. It is now a
    // concrete requirement of the selected factory, so its feature is required.
    assert!(
        compile_members(
            &registry,
            &owner,
            USER_MOUNT,
            &[MEMBERS[0]],
            &[TOOL_FEATURE, RESOURCE_FEATURE]
        )
        .is_err()
    );
    let snapshot = compile_members(
        &registry,
        &owner,
        USER_MOUNT,
        &[MEMBERS[0]],
        &[TOOL_FEATURE, RESOURCE_FEATURE, UNUSED_FEATURE],
    )
    .unwrap()
    .with_target_resource_bindings(&owner, vec![binding(&owner)])
    .unwrap();
    assert_eq!(snapshot.content().contributions().count(), 1);
    assert_eq!(snapshot.content().enabled_capabilities.len(), 2);
    assert!(
        !snapshot
            .resolved_capability(&CONTEXT_ID.into())
            .unwrap()
            .consumption
            .is_contribution()
    );
    assert!(snapshot.resolved_capability(&MEMBERS[2].into()).is_none());
    assert!(snapshot.resolved_capability(&RESOURCE_ID.into()).is_none());
    assert_eq!(
        snapshot
            .resolved_capability(&MEMBERS[0].into())
            .unwrap()
            .dependency_refs,
        vec![CapabilityRef {
            id: CONTEXT_ID.into(),
            version: VERSION.into()
        }]
    );
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    let mut request = invoke_request(&snapshot, &owner);
    request.resource_binding_ids = snapshot
        .policy(&MEMBERS[0].into())
        .unwrap()
        .resource_binding_ids
        .clone();
    let result = kernel.invoke(&snapshot, &active, request).await.unwrap();
    assert_eq!(
        result.0["resourceBindings"][0]["parameters"]["index"],
        "user-local-index"
    );
    kernel
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        host.stop_generation(generation).await.unwrap();
    }
}

fn contract_tool_resource(
    builtin: &mut PluginRegistration,
    kind: &str,
    serialized: bool,
) -> ExactRoleContractRef {
    let mut manifest = builtin.metadata.manifest.payload.clone();
    let tool = manifest
        .contributions
        .capabilities
        .iter_mut()
        .find(|value| value.id.as_ref() == MEMBERS[0])
        .unwrap();
    tool.contributions.resource_kinds = BTreeSet::from([kind.into()]);
    let tool_digest = digest_payload(tool).unwrap();
    let contract = &mut manifest.contributions.role_contracts[0];
    contract
        .members
        .iter_mut()
        .find(|member| member.capability.id.as_ref() == MEMBERS[0])
        .unwrap()
        .capability_manifest_digest = tool_digest;
    contract.serialized_target_resource_kind = serialized.then(|| kind.into());
    let reference = ExactRoleContractRef {
        key: contract.key.clone(),
        contract_digest: digest_payload(contract).unwrap(),
    };
    manifest.contributions.role_providers[0].role = reference.clone();
    manifest.contributions.role_providers[0]
        .members
        .get_mut(&MEMBERS[0].into())
        .unwrap()
        .required_resource_kinds = BTreeSet::from([kind.into()]);
    builtin.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    reference
}

#[tokio::test]
async fn selected_provider_replaces_private_resource_requirements_instead_of_unioning_defaults() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    let mut builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
    let contract = contract_tool_resource(&mut builtin, "fixture.remote-credential", false);
    let requirement = with_requirements(&original, false);
    let mut manifest = requirement.manifest.payload;
    manifest.package.contributions.role_providers[0].role = contract;
    let artifact =
        PluginPackageArtifactV1::new(requirement.artifact_id, manifest, requirement.files).unwrap();
    let implementation = adapter(artifact, &temp);
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin,
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let default = compile_members(&registry, &owner, BUILTIN_MOUNT, &[MEMBERS[0]], &[]).unwrap();
    assert_eq!(
        default.content().required_resource_kinds,
        BTreeSet::from(["fixture.remote-credential".into()])
    );
    let selected = compile_members(
        &registry,
        &owner,
        USER_MOUNT,
        &[MEMBERS[0]],
        &[TOOL_FEATURE, RESOURCE_FEATURE],
    )
    .unwrap()
    .with_target_resource_bindings(&owner, vec![binding(&owner)])
    .unwrap();
    assert_eq!(
        selected.content().required_resource_kinds,
        BTreeSet::from([RESOURCE_KIND.into()])
    );
    assert_eq!(
        selected
            .policy(&MEMBERS[0].into())
            .unwrap()
            .required_resource_kinds,
        selected.content().required_resource_kinds
    );
    let active = SessionCapabilityState::new(&selected).snapshot().unwrap();
    let mut request = invoke_request(&selected, &owner);
    request.resource_binding_ids = selected
        .policy(&MEMBERS[0].into())
        .unwrap()
        .resource_binding_ids
        .clone();
    let result = kernel.invoke(&selected, &active, request).await.unwrap();
    assert_eq!(
        result.0["resourceBindings"][0]["parameters"]["index"],
        "user-local-index"
    );
    kernel
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        host.stop_generation(generation).await.unwrap();
    }
}

#[tokio::test]
async fn implementation_cannot_change_typed_resource_output_or_remove_serialized_target() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    for serialized in [false, true] {
        let mut builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
        let mut manifest = original.manifest.payload.clone();
        if serialized {
            manifest.package.contributions.role_providers[0].role =
                contract_tool_resource(&mut builtin, RESOURCE_KIND, true);
        } else {
            manifest
                .package
                .contributions
                .capabilities
                .iter_mut()
                .find(|value| value.id.as_ref() == RESOURCE_ID)
                .unwrap()
                .contributions
                .resource_kinds = BTreeSet::from(["fixture.other-kind".into()]);
            manifest.package.contributions.role_providers[0]
                .members
                .get_mut(&MEMBERS[2].into())
                .unwrap()
                .required_resource_kinds = BTreeSet::from(["fixture.other-kind".into()]);
        }
        let artifact = PluginPackageArtifactV1::new(
            original.artifact_id.clone(),
            manifest,
            original.files.clone(),
        )
        .unwrap();
        let implementation = adapter(artifact, &temp);
        assert!(matches!(
            requirements_registry().replace_all(vec![
                builtin,
                implementation.registration(host.clone()).unwrap()
            ]),
            Err(KernelError::InvalidRoleProvider { .. })
        ));
    }
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn selected_js_resource_requirements_are_compiled_and_consumed_without_public_internal_capabilities()
 {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let builtin = builtin(&baseline, host.clone(), calls.clone());
    let implementation = adapter(with_requirements(&original, false), &temp);
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin.clone(),
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };

    let default = compile_members(&registry, &owner, BUILTIN_MOUNT, &[MEMBERS[0]], &[]).unwrap();
    assert!(default.content().required_resource_kinds.is_empty());
    assert!(
        default.content().required_runtime_features.is_empty(),
        "unselected implementations add no requirements"
    );
    let default_active = SessionCapabilityState::new(&default).snapshot().unwrap();
    assert_eq!(
        kernel
            .invoke(&default, &default_active, invoke_request(&default, &owner))
            .await
            .unwrap()
            .0["builtin"],
        true
    );

    let snapshot = compile_members(
        &registry,
        &owner,
        USER_MOUNT,
        &[MEMBERS[0]],
        &[TOOL_FEATURE, RESOURCE_FEATURE],
    )
    .unwrap();
    assert_eq!(
        snapshot.content().required_resource_kinds,
        BTreeSet::from([RESOURCE_KIND.into()])
    );
    assert_eq!(
        snapshot.content().required_runtime_features,
        BTreeSet::from([TOOL_FEATURE.into(), RESOURCE_FEATURE.into()])
    );
    assert_eq!(
        snapshot.content().capability_allowlist,
        BTreeSet::from([MEMBERS[0].into()])
    );
    assert_eq!(snapshot.content().enabled_capabilities.len(), 1);
    assert_eq!(
        snapshot.content().enabled_capabilities[0].required_resource_kinds,
        snapshot.content().required_resource_kinds
    );
    assert_eq!(
        snapshot.content().enabled_capabilities[0].required_runtime_features,
        snapshot.content().required_runtime_features
    );
    assert_eq!(snapshot.authority_policies.len(), 1);
    assert!(
        snapshot.policy(&MEMBERS[2].into()).is_none(),
        "internal resource export is not a public grant"
    );
    assert!(snapshot.resolved_capability(&TOOL_ID.into()).is_none());
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    assert!(matches!(
        kernel
            .invoke(&snapshot, &active, invoke_request(&snapshot, &owner))
            .await,
        Err(KernelError::CapabilityResourceNotBound { .. })
    ));
    assert_eq!(
        host.process_count(),
        0,
        "missing resources must fail before starting JS"
    );
    let mut foreign = binding(&owner);
    foreign.owner_id = "another-owner".into();
    assert!(matches!(
        snapshot
            .clone()
            .with_target_resource_bindings(&owner, vec![foreign]),
        Err(KernelError::ResourceOwnerMismatch { .. })
    ));

    let snapshot = snapshot
        .with_target_resource_bindings(&owner, vec![binding(&owner)])
        .unwrap();
    let mut request = invoke_request(&snapshot, &owner);
    request.resource_binding_ids = snapshot
        .policy(&MEMBERS[0].into())
        .unwrap()
        .resource_binding_ids
        .clone();
    let mut forbidden = request.clone();
    forbidden.capability_id = TOOL_ID.into();
    assert!(matches!(
        kernel.invoke(&snapshot, &active, forbidden).await,
        Err(KernelError::CapabilityNotInPreset { .. })
    ));
    let result = kernel
        .invoke(&snapshot, &active, request.clone())
        .await
        .unwrap();
    assert_eq!(
        result.0["resourceBindings"][0]["parameters"]["index"],
        "user-local-index"
    );
    assert_eq!(
        result.0["resourceBindings"][0]["bindingId"],
        "fixture-index-binding"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "JS replacement must not invoke builtin"
    );

    // Removing target bindings revokes access even while a cached resource exists.
    let revoked = snapshot
        .clone()
        .with_target_resource_bindings(&owner, Vec::new())
        .unwrap();
    assert!(
        kernel
            .invoke(&revoked, &active, request.clone())
            .await
            .is_err()
    );
    assert!(
        kernel
            .invoke(&revoked, &active, invoke_request(&revoked, &owner))
            .await
            .is_err()
    );
    kernel
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();

    // Same Mount/Role, but a different exact artifact and requirement set.
    let changed = kernel
        .replace_all(vec![builtin, baseline.registration(host.clone()).unwrap()])
        .unwrap();
    assert!(kernel.invoke(&snapshot, &active, request).await.is_err());
    let new_snapshot = compile_members(&changed, &owner, USER_MOUNT, &[MEMBERS[0]], &[]).unwrap();
    assert!(new_snapshot.content().required_resource_kinds.is_empty());
    assert_ne!(snapshot.snapshot_ref(), new_snapshot.snapshot_ref());
    assert_eq!(
        snapshot.content().required_resource_kinds,
        BTreeSet::from([RESOURCE_KIND.into()])
    );
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        host.stop_generation(generation).await.unwrap();
    }
}

#[tokio::test]
async fn implicit_resource_factory_availability_is_checked_without_selecting_unrelated_members() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    let builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
    let requirements = with_requirements(&original, false);
    let implementation = adapter(requirements.clone(), &temp);
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin.clone(),
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    assert!(
        matches!(compile_members(&registry, &owner, USER_MOUNT, &[MEMBERS[0]], &[TOOL_FEATURE]), Err(KernelError::RuntimeFeatureUnavailable { feature, .. }) if feature == RESOURCE_FEATURE)
    );
    assert!(
        compile_members(
            &registry,
            &owner,
            USER_MOUNT,
            &[MEMBERS[0]],
            &[TOOL_FEATURE, RESOURCE_FEATURE]
        )
        .is_ok()
    );

    let mut manifest = requirements.manifest.payload.clone();
    manifest.package.contributions.role_providers[0]
        .members
        .get_mut(&MEMBERS[2].into())
        .unwrap()
        .supported_platforms = vec![PlatformConstraint::Targets {
        host_targets: BTreeSet::from(["aarch64-apple-darwin".into()]),
        host_surfaces: BTreeSet::new(),
    }];
    let incompatible =
        PluginPackageArtifactV1::new(requirements.artifact_id, manifest, requirements.files)
            .unwrap();
    let incompatible = adapter(incompatible, &temp);
    let registry = kernel
        .replace_all(vec![
            builtin,
            incompatible.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    assert!(
        matches!(compile_members(&registry, &owner, USER_MOUNT, &[MEMBERS[0]], &[TOOL_FEATURE, RESOURCE_FEATURE]), Err(KernelError::CapabilityUnavailableOnPlatform { capability_id, .. }) if capability_id.as_ref() == MEMBERS[2])
    );
    assert_eq!(host.process_count(), 0);
}

#[tokio::test]
async fn context_and_non_agent_operations_consume_the_selected_resource_requirement() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    let builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
    let implementation = adapter(with_requirements(&original, true), &temp);
    let kernel = requirements_registry();
    let registry = kernel
        .replace_all(vec![
            builtin,
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let snapshot = compile_members(
        &registry,
        &owner,
        USER_MOUNT,
        &[MEMBERS[1]],
        &[UNUSED_FEATURE, RESOURCE_FEATURE],
    )
    .unwrap()
    .with_target_resource_bindings(&owner, vec![binding(&owner)])
    .unwrap();
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    let context = kernel
        .contribute_context(
            &snapshot,
            &active,
            access(&snapshot, active.generation, &owner, MEMBERS[1]),
        )
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(
        context.0["resourceBindings"][0]["parameters"]["index"],
        "user-local-index"
    );
    assert_eq!(
        snapshot.content().capability_allowlist,
        BTreeSet::from([MEMBERS[1].into()])
    );
    kernel
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();

    let mut env = environment(registry.registry_digest.clone());
    env.available_runtime_features = [TOOL_FEATURE, RESOURCE_FEATURE]
        .into_iter()
        .map(Into::into)
        .collect();
    let selection = selection(&registry, USER_MOUNT);
    let members = BTreeSet::from([MEMBERS[0].into()]);
    assert!(matches!(
        resolve_exact_role_provider_lock(
            &registry,
            &ROLE.into(),
            &selection,
            &members,
            &BTreeMap::new(),
            &env
        ),
        Err(KernelError::CapabilityResourceNotBound { .. })
    ));
    let resource = binding(&owner);
    let lock = resolve_exact_role_provider_lock(
        &registry,
        &ROLE.into(),
        &selection,
        &members,
        &BTreeMap::from([(resource.binding_id.clone(), resource.clone())]),
        &env,
    )
    .unwrap();
    let result = kernel
        .invoke_role_tool(RoleToolOperationRequest {
            member: RoleMemberInvocationRequest {
                principal: owner.clone(),
                session_owner: owner.clone(),
                turn_id: None,
                operation_id: "resource-operation".into(),
                correlation_id: "resource-correlation".into(),
                capability_id: MEMBERS[0].into(),
                resource_binding_ids: BTreeSet::from([resource.binding_id.clone()]),
                state_scope_key: "operation:resource".into(),
                admission: RoleMemberAdmission::Operation {
                    provider_lock: lock,
                    registry_generation: registry.generation,
                    registry_digest: registry.registry_digest.clone(),
                    resource_bindings: vec![resource],
                },
            },
            action_id: TOOL_ACTION.into(),
            idempotency_key: "resource-operation".into(),
            input: StrictJsonValue(json!({})),
        })
        .await
        .unwrap();
    assert_eq!(
        result.0["resourceBindings"][0]["parameters"]["index"],
        "user-local-index"
    );
    kernel
        .release_resources(&"session:fixture-session".into())
        .await
        .unwrap();
    kernel
        .release_resources(&"operation:resource".into())
        .await
        .unwrap();
    if let JavaScriptHostState::Running { generation, .. } = host.state() {
        host.stop_generation(generation).await.unwrap();
    }
}
