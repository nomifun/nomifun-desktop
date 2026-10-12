//! Optional physical readiness must not weaken publication or execution checks.
use super::*;
use nomifun_agent_contracts::{
    CapabilityActionDescriptor, EffectClass, ResourceKind, ToolPresentationKind,
};
use nomifun_agent_kernel::{CompileResourceCeilingPolicy, OptionalResourceCeiling};

const RESOURCE: &str = "fixture_device";
const ACTION: &str = "device/observe";

fn optional_fixture() -> (Fixture, CompileResourceCeilingPolicy) {
    let mut fixture = Fixture::new();
    let capability = fixture.registry.capabilities.get_mut(&MEMBER.into()).unwrap();
    capability.manifest.kind = CapabilityKind::Tool;
    capability.manifest.contributions = CapabilityContributions {
        actions: vec![CapabilityActionDescriptor {
            action_id: ACTION.into(),
            input_schema: "schema://platform.test/device-input@1".into(),
            output_schema: "schema://platform.test/device-output@1".into(),
            effect_class: EffectClass::ReadLocal,
            presentation: ToolPresentationKind::FunctionTool,
            required_resource_operations: Some(BTreeMap::from([(
                ResourceKind::from(RESOURCE), BTreeSet::from(["observe".into()]),
            )])),
        }],
        resource_kinds: BTreeSet::from([RESOURCE.into()]),
        ..Default::default()
    };
    capability.schema_digest = digest_payload(&capability.manifest).unwrap();
    capability.contribution_lock.contract_digest = capability.schema_digest.clone();
    let capability = capability.clone();
    let contract = fixture.registry.role_contracts.get_mut(&ROLE.into()).unwrap();
    contract.manifest.members[0].capability_manifest_digest = capability.schema_digest.clone();
    contract.contract_digest = digest_payload(&contract.manifest).unwrap();
    let exact_role = ExactRoleContractRef {
        key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone(),
    };
    for provider in fixture.registry.role_providers.values_mut() {
        provider.provider.role = exact_role.clone();
        provider.contribution.role = exact_role.clone();
        let member = provider.contribution.members.get_mut(&MEMBER.into()).unwrap();
        member.required_resource_kinds = BTreeSet::from([RESOURCE.into()]);
        member.supported_platforms = vec![PlatformConstraint::Targets {
            host_targets: BTreeSet::from(["test".into()]),
            host_surfaces: BTreeSet::from(["desktop".into()]),
        }];
        provider.provider.contribution_digest = digest_payload(&provider.contribution).unwrap();
    }
    fixture.environment.host_surface = "headless".into();
    fixture.environment.installation_role_bindings.get_mut(&ROLE.into()).unwrap().selection =
        RoleProviderSelection { role: exact_role, provider_mount_id: "builtin".into() };
    fixture.draft.document.enabled_capabilities[0].action_allowlist = BTreeSet::from([ACTION.into()]);
    fixture.catalog.capabilities = vec![capability.clone()];
    fixture.catalog.role_contracts = fixture.registry.role_contracts.values().cloned().collect();
    fixture.catalog.role_providers = fixture.registry.role_providers.values().cloned().collect();
    let entry = CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
        manifest: capability.manifest.clone(),
        provenance: CapabilityProvenance {
            owner: CapabilityOwner::Package { package: capability.manifest.package.clone() },
            source_kind: capability.contribution_lock.source_kind,
            source_identity: capability.contribution_lock.source_identity.clone(),
            mount_id: None, mcp_binding_id: None,
            artifact_digest: Some(capability.target_artifact_digest.clone()),
        },
        publication_state: CapabilityPublicationState::Active,
        availability: BTreeMap::from([(CapabilityConsumer::Agent, CatalogAvailability::Active)]),
    }).unwrap();
    fixture.catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
    let policy = CompileResourceCeilingPolicy {
        optional_resources: BTreeMap::from([(MEMBER.into(), OptionalResourceCeiling {
            resource_kind: RESOURCE.into(),
            provider: fixture.registry.role_providers[&(ROLE.into(), "builtin".into())].provider.clone(),
        })]),
    };
    assert_eq!(nomifun_agent_kernel::deferred_resource_ceiling_capabilities(&fixture.registry, &policy),
        BTreeSet::from([MEMBER.into()]), "fixture must prove exact optional-resource eligibility");
    (fixture, policy)
}

fn set_availability(fixture: &mut Fixture, availability: CatalogAvailability) {
    fixture.catalog.unavailable_capabilities = availability_code(&availability)
        .map(|code| BTreeMap::from([(MEMBER.into(), code)])).unwrap_or_default();
    fixture.catalog.formal_capability_entries.get_mut(&CapabilityRef { id: MEMBER.into() }).unwrap()
        .availability.insert(CapabilityConsumer::Agent, availability);
}

fn compile_with_policy(
    fixture: &Fixture,
    policy: &CompileResourceCeilingPolicy,
    saved: Option<&(AgentPresetRevision, ResolvedSnapshotEnvelope)>,
) -> Result<PresetCompilation, ControlPlaneError> {
    fixture.compiler().with_resource_ceiling_policy(policy.clone()).compile(
        &"owner".into(), &fixture.draft, saved.map(|s| &s.0), saved.map(|s| &s.1), &fixture.catalog,
    )
}

fn saved_compilation(compiled: PresetCompilation) -> (AgentPresetRevision, ResolvedSnapshotEnvelope) {
    assert!(compiled.diagnostics.is_empty(), "{:?}", compiled.diagnostics);
    let snapshot = compiled.snapshot.unwrap();
    (AgentPresetRevision {
        reference: compiled.candidate_revision_ref,
        payload: compiled.payload,
        contribution_locks: compiled.contribution_locks,
        created_by: "owner".into(), created_at_ms: snapshot.created_at_ms, reason: None,
    }, snapshot)
}

#[test]
fn optional_resource_ceiling_precheck_defers_only_generic_readiness_for_new_and_reused_plans() {
    let (mut fixture, policy) = optional_fixture();
    set_availability(&mut fixture, CatalogAvailability::Unavailable { reason: "CAPABILITY_UNAVAILABLE".into() });
    let strict = fixture.compile(None);
    assert!(strict.snapshot.is_none());
    assert_eq!(strict.diagnostics.iter().map(|d| d.code.as_str()).collect::<Vec<_>>(), vec!["CAPABILITY_UNAVAILABLE"]);

    let saved = saved_compilation(compile_with_policy(&fixture, &policy, None).unwrap());
    let reused = compile_with_policy(&fixture, &policy, Some(&saved)).unwrap();
    assert!(reused.diagnostics.is_empty(), "{:?}", reused.diagnostics);
    assert_eq!(reused.candidate_revision_ref, saved.0.reference);
    assert_eq!(reused.snapshot, Some(saved.1.clone()), "physical readiness cannot rewrite saved authority");
    let strict_reuse = fixture.compile(Some(&saved));
    assert!(strict_reuse.snapshot.is_none(), "an optional save must not bypass a later strict caller");
}

#[test]
fn optional_resource_ceiling_precheck_keeps_specific_failures_blocking() {
    let (mut fixture, policy) = optional_fixture();
    let saved = saved_compilation(compile_with_policy(&fixture, &policy, None).unwrap());
    let contract_digest = fixture.registry.capabilities[&MEMBER.into()].schema_digest.clone();
    for (availability, expected_code) in [
        (CatalogAvailability::Unavailable { reason: "CAPABILITY_OWNER_MISMATCH".into() }, "CAPABILITY_OWNER_MISMATCH"),
        (CatalogAvailability::Disabled { reason: "CAPABILITY_DISABLED".into() }, "CAPABILITY_DISABLED"),
        (CatalogAvailability::NeedsRuntime { missing_runtime_features: BTreeSet::from(["fixture-runtime".into()]) }, "CAPABILITY_NEEDS_RUNTIME"),
        (CatalogAvailability::ContractMismatch { expected_contract_digest: contract_digest, actual_contract_digest: None }, "CAPABILITY_CONTRACT_MISMATCH"),
    ] {
        set_availability(&mut fixture, availability);
        for prior in [None, Some(&saved)] {
            let compiled = compile_with_policy(&fixture, &policy, prior).unwrap();
            assert!(compiled.snapshot.is_none(), "{expected_code} must block even an eligible optional resource");
            assert_eq!(compiled.diagnostics.iter().map(|d| d.code.as_str()).collect::<Vec<_>>(), vec![expected_code]);
        }
    }
}

#[test]
fn optional_resource_ceiling_still_requires_formal_publication_before_reuse() {
    let (mut fixture, policy) = optional_fixture();
    let saved = saved_compilation(compile_with_policy(&fixture, &policy, None).unwrap());
    fixture.catalog.formal_capability_entries.clear();
    for prior in [None, Some(&saved)] {
        let error = compile_with_policy(&fixture, &policy, prior).unwrap_err();
        assert_eq!(error.code().as_ref(), "CAPABILITY_CATALOG_INVALID");
        assert!(error.to_string().contains("no formal Catalog entry"), "{error}");
    }
}

#[test]
fn optional_resource_ceiling_reaches_installation_defaults_and_frozen_session_compilation() {
    let (fixture, policy) = optional_fixture();
    let selection = &fixture.environment.installation_role_bindings[&ROLE.into()].selection;
    assert!(fixture.compiler().validate_role_default(selection).is_err());
    fixture.compiler().with_resource_ceiling_policy(policy.clone()).validate_role_default(selection).unwrap();

    let saved = saved_compilation(compile_with_policy(&fixture, &policy, None).unwrap());
    assert!(fixture.compiler().compile_session_configuration(
        &"owner".into(), "strict-session".into(), saved.0.payload.clone(), &saved.1, &fixture.catalog, "strict",
    ).is_err());
    let (revision, updated) = fixture.compiler().with_resource_ceiling_policy(policy).compile_session_configuration(
        &"owner".into(), "optional-session".into(), saved.0.payload.clone(), &saved.1, &fixture.catalog, "optional",
    ).unwrap();
    let mut expected = saved.1.content.clone();
    expected.preset_revision_ref = revision.reference;
    assert_eq!(updated.content, expected);
}

#[test]
fn exported_kernel_resource_ceiling_entrypoints_preserve_default_strict_behavior() {
    let (fixture, policy) = optional_fixture();
    let saved = saved_compilation(compile_with_policy(&fixture, &policy, None).unwrap());
    let request = saved_snapshot_request(saved.0.clone(), &saved.1);
    assert!(KernelAgentPresetCompiler::compile(&fixture.registry, &fixture.environment, request.clone()).is_err());
    let compiled = KernelAgentPresetCompiler::compile_with_resource_ceiling(
        &fixture.registry, &fixture.environment, request.clone(), &policy,
    ).unwrap();
    assert_eq!(compiled.non_executable_capability_ids, BTreeSet::from([MEMBER.into()]));
    assert!(KernelAgentPresetCompiler::compile_with_frozen_role_providers(
        &fixture.registry, &fixture.environment, request.clone(), &saved.1,
    ).is_err());
    let frozen = KernelAgentPresetCompiler::compile_with_frozen_role_providers_and_resource_ceiling(
        &fixture.registry, &fixture.environment, request, &saved.1, &policy,
    ).unwrap();
    assert_eq!(frozen.envelope, compiled.envelope);
    assert!(!KernelAgentPresetCompiler::role_providers_unchanged(&fixture.registry, &fixture.environment, &saved.0, &saved.1));
    assert!(KernelAgentPresetCompiler::role_providers_unchanged_with_resource_ceiling(
        &fixture.registry, &fixture.environment, &saved.0, &saved.1, &policy,
    ));
    let selection = &fixture.environment.installation_role_bindings[&ROLE.into()].selection;
    assert!(KernelAgentPresetCompiler::validate_role_default(&fixture.registry, &fixture.environment, selection).is_err());
    KernelAgentPresetCompiler::validate_role_default_with_resource_ceiling(
        &fixture.registry, &fixture.environment, selection, &policy, "desktop",
    ).unwrap();
}
