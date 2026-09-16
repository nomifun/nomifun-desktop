//! Selected implementation conflicts are compilation constraints, not grants.
use super::*;
use nomifun_agent_contracts::CapabilityConflict;

struct ConflictFixture {
    _temp: TempDir,
    host: Arc<ExtensionHostSupervisor>,
    kernel: KernelRegistry,
    registry: Arc<MaterializedRegistry>,
    owner: PrincipalRef,
    builtin: PluginRegistration,
}

impl ConflictFixture {
    async fn new(conflicts: &[(&str, &str)], tool_uses_resource: bool) -> Self {
        let main = std::fs::read(fixture("main.mjs")).unwrap();
        let temp = TempDir::new().unwrap();
        let original = mapped_artifact(&main);
        let baseline = adapter(original.clone(), &temp);
        let host = host().await;
        let builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
        let mut manifest = original.manifest.payload;
        for (source, target) in conflicts {
            manifest
                .package
                .contributions
                .capabilities
                .iter_mut()
                .find(|capability| capability.id.as_ref() == *source)
                .unwrap()
                .conflicts
                .push(conflict(target));
        }
        if tool_uses_resource {
            manifest
                .package
                .contributions
                .capabilities
                .iter_mut()
                .find(|capability| capability.id.as_ref() == TOOL_ID)
                .unwrap()
                .contributions
                .resource_kinds
                .insert(RESOURCE_KIND.into());
            manifest.package.contributions.role_providers[0]
                .members
                .get_mut(&MEMBERS[0].into())
                .unwrap()
                .required_resource_kinds
                .insert(RESOURCE_KIND.into());
        }
        let artifact =
            PluginPackageArtifactV1::new(original.artifact_id, manifest, original.files).unwrap();
        let implementation = adapter(artifact, &temp);
        let kernel = registry();
        let registry = kernel
            .replace_all(vec![
                builtin.clone(),
                implementation.registration(host.clone()).unwrap(),
            ])
            .expect(
                "different conflict declarations are compatible with the same callable contract",
            );
        Self {
            _temp: temp,
            host,
            kernel,
            registry,
            builtin,
            owner: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: "fixture-owner".into(),
            },
        }
    }

    fn compile(&self, mount: &str, members: &[&str]) -> Result<CompiledSnapshot, KernelError> {
        compile_members(&self.registry, &self.owner, mount, members, &[])
    }

    async fn stop(&self) {
        if let JavaScriptHostState::Running { generation, .. } = self.host.state() {
            self.host.stop_generation(generation).await.unwrap();
        }
    }
}

fn conflict(target: &str) -> CapabilityConflict {
    CapabilityConflict {
        capability: CapabilityRef {
            id: target.into(),
            version: VERSION.into(),
        },
        reason: "fixture incompatible implementation".into(),
    }
}

fn assert_conflict(result: Result<CompiledSnapshot, KernelError>, left: &str, right: &str) {
    assert!(
        matches!(result, Err(KernelError::CapabilityConflict { left: l, right: r })
        if l.as_ref() == left && r.as_ref() == right),
        "expected conflict {left} -> {right}"
    );
}

#[tokio::test]
async fn selected_js_conflict_is_enforced_without_blocking_other_provider_or_granting_implementation()
 {
    let f = ConflictFixture::new(&[(TOOL_ID, MEMBERS[1])], false).await;
    AgentPresetCompiler::validate_role_default(
        &f.registry,
        &environment(f.registry.registry_digest.clone()),
        &selection(&f.registry, USER_MOUNT),
    )
    .expect("setting a default is not a request to consume all members together");
    let builtin = f.compile(BUILTIN_MOUNT, &[MEMBERS[0], MEMBERS[1]]).unwrap();
    assert_eq!(builtin.content().capability_allowlist.len(), 2);
    assert_conflict(
        f.compile(USER_MOUNT, &[MEMBERS[0], MEMBERS[1]]),
        TOOL_ID,
        MEMBERS[1],
    );
    assert_eq!(
        f.host.process_count(),
        0,
        "compile rejection must have no JS effects"
    );

    let selected = f.compile(USER_MOUNT, &[MEMBERS[0]]).unwrap();
    let default = f.compile(BUILTIN_MOUNT, &[MEMBERS[0]]).unwrap();
    assert_eq!(
        selected.content().capability_allowlist,
        default.content().capability_allowlist
    );
    assert_eq!(selected.authority_policies, default.authority_policies);
    assert!(selected.resolved_capability(&TOOL_ID.into()).is_none());
    assert!(selected.policy(&TOOL_ID.into()).is_none());
    let active = SessionCapabilityState::new(&selected).snapshot().unwrap();
    let result = f
        .kernel
        .invoke(&selected, &active, invoke_request(&selected, &f.owner))
        .await
        .unwrap();
    assert_eq!(result.0["contributionId"], "fixture.tool.contribution");
    let mut internal = invoke_request(&selected, &f.owner);
    internal.capability_id = TOOL_ID.into();
    assert!(f.kernel.invoke(&selected, &active, internal).await.is_err());
    f.kernel.replace_all(vec![f.builtin.clone()]).unwrap();
    assert!(
        f.kernel
            .invoke(&selected, &active, invoke_request(&selected, &f.owner))
            .await
            .is_err(),
        "withdrawn selected implementation must not fall back to builtin"
    );
    f.stop().await;
}

#[tokio::test]
async fn reverse_conflict_can_target_an_internal_selected_implementation() {
    let f = ConflictFixture::new(&[(CONTEXT_ID, TOOL_ID)], false).await;
    f.compile(USER_MOUNT, &[MEMBERS[1]]).unwrap();
    f.compile(BUILTIN_MOUNT, &[MEMBERS[0], MEMBERS[1]]).unwrap();
    assert_conflict(
        f.compile(USER_MOUNT, &[MEMBERS[0], MEMBERS[1]]),
        CONTEXT_ID,
        TOOL_ID,
    );
    assert_eq!(f.host.process_count(), 0);
}

#[tokio::test]
async fn unused_context_and_resource_members_do_not_constrain_selected_tool() {
    let f = ConflictFixture::new(&[(CONTEXT_ID, TOOL_ID), (RESOURCE_ID, TOOL_ID)], false).await;
    let selected = f.compile(USER_MOUNT, &[MEMBERS[0]]).unwrap();
    assert_eq!(selected.content().enabled_capabilities.len(), 1);
    assert_eq!(selected.authority_policies.len(), 1);
    let active = SessionCapabilityState::new(&selected).snapshot().unwrap();
    let result = f
        .kernel
        .invoke(&selected, &active, invoke_request(&selected, &f.owner))
        .await
        .unwrap();
    assert_eq!(result.0["contributionId"], "fixture.tool.contribution");
    f.stop().await;
}

#[tokio::test]
async fn implicit_resource_conflict_is_checked_for_agent_and_non_agent_admission() {
    let f = ConflictFixture::new(&[(RESOURCE_ID, TOOL_ID)], true).await;
    f.compile(BUILTIN_MOUNT, &[MEMBERS[0]]).unwrap();
    assert_conflict(f.compile(USER_MOUNT, &[MEMBERS[0]]), RESOURCE_ID, TOOL_ID);
    let selected = selection(&f.registry, USER_MOUNT);
    let result = resolve_exact_role_provider_lock(
        &f.registry,
        &ROLE.into(),
        &selected,
        &BTreeSet::from([MEMBERS[0].into()]),
        &BTreeMap::from([(binding(&f.owner).binding_id.clone(), binding(&f.owner))]),
        &environment(f.registry.registry_digest.clone()),
    );
    assert!(
        matches!(result, Err(KernelError::CapabilityConflict { left, right })
        if left.as_ref() == RESOURCE_ID && right.as_ref() == TOOL_ID)
    );
    assert_eq!(
        f.host.process_count(),
        0,
        "conflicts precede resource acquisition"
    );
}

#[tokio::test]
async fn facade_conflicts_cannot_be_removed_by_a_mapped_provider() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let original = mapped_artifact(&main);
    let baseline = adapter(original.clone(), &temp);
    let host = host().await;
    let mut builtin = builtin(&baseline, host.clone(), Arc::new(AtomicUsize::new(0)));
    let mut package = builtin.metadata.manifest.payload;
    let facade = package
        .contributions
        .capabilities
        .iter_mut()
        .find(|capability| capability.id.as_ref() == MEMBERS[0])
        .unwrap();
    facade.conflicts.push(conflict(MEMBERS[1]));
    let manifest_digest = digest_payload(facade).unwrap();
    let contract = &mut package.contributions.role_contracts[0];
    contract
        .members
        .iter_mut()
        .find(|member| member.capability.id.as_ref() == MEMBERS[0])
        .unwrap()
        .capability_manifest_digest = manifest_digest;
    let exact = ExactRoleContractRef {
        key: contract.key.clone(),
        contract_digest: digest_payload(contract).unwrap(),
    };
    package.contributions.role_providers[0].role = exact.clone();
    builtin.metadata.manifest = ArtifactEnvelope::new(package).unwrap();
    let mut manifest = original.manifest.payload;
    manifest.package.contributions.role_providers[0].role = exact;
    let artifact =
        PluginPackageArtifactV1::new(original.artifact_id, manifest, original.files).unwrap();
    let implementation = adapter(artifact, &temp);
    let kernel = registry();
    let materialized = kernel
        .replace_all(vec![
            builtin,
            implementation.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    for mount in [BUILTIN_MOUNT, USER_MOUNT] {
        compile_members(&materialized, &owner, mount, &[MEMBERS[0]], &[]).unwrap();
        assert_conflict(
            compile_members(&materialized, &owner, mount, &[MEMBERS[0], MEMBERS[1]], &[]),
            MEMBERS[0],
            MEMBERS[1],
        );
    }
    assert_eq!(host.process_count(), 0);
}
