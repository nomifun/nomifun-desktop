use super::*;

fn fixture() -> (KernelRegistry, crate::CompiledSnapshot, PrincipalRef) {
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    ).unwrap();
    let materialized = registry.replace_all(vec![sample_registration("preflight:")]).unwrap();
    let owner = principal("preflight-owner");
    let compiled = AgentPresetCompiler::compile(
        &materialized,
        &compiler_environment(materialized.registry_digest.clone()),
        compile_request(sample_revision(&owner.principal_id), owner.clone()),
    ).unwrap().with_target_resource_bindings(
        &owner, vec![resource_binding(&owner.principal_id)],
    ).unwrap();
    (registry, compiled, owner)
}

#[tokio::test]
async fn preflight_never_dispatches_and_invoke_rechecks_owner_authority() {
    let (registry, compiled, owner) = fixture();
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
    let request = invocation(&compiled, owner, active.generation, "hello");
    for _ in 0..3 {
        registry.preflight_invocation(&compiled, &active, &request).unwrap();
    }
    let mut revoked = active.clone();
    revoked.active.clear();
    assert!(matches!(
        registry.invoke(&compiled, &revoked, request.clone()).await,
        Err(KernelError::CapabilityNotActive { .. })
    ));
    let result = registry.invoke(&compiled, &active, request).await.unwrap();
    assert_eq!(result.0, json!({"echo": "preflight:hello", "count": 1}));
}

#[tokio::test]
async fn preflight_rejects_owner_action_resource_and_active_drift_without_dispatch() {
    let (registry, compiled, owner) = fixture();
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
    let request = invocation(&compiled, owner, active.generation, "hello");
    for mutation in ["owner", "action", "binding", "generation", "unknown"] {
        let mut denied = request.clone();
        match mutation {
            "owner" => denied.principal = principal("other-owner"),
            "action" => denied.action_id = ActionId::from("not-declared"),
            "binding" => denied.resource_binding_ids.clear(),
            "generation" => denied.active_set_generation += 1,
            _ => denied.capability_id = CapabilityId::from("unknown"),
        }
        assert!(registry.preflight_invocation(&compiled, &active, &denied).is_err(), "{mutation}");
        assert!(registry.invoke(&compiled, &active, denied).await.is_err(), "{mutation}");
    }
    let result = registry.invoke(&compiled, &active, request).await.unwrap();
    assert_eq!(result.0["count"], json!(1));
}

#[tokio::test]
async fn preflight_and_dispatch_reject_republished_exact_target() {
    let (registry, compiled, owner) = fixture();
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
    let request = invocation(&compiled, owner, active.generation, "hello");
    registry.preflight_invocation(&compiled, &active, &request).unwrap();
    registry.replace_all(vec![registration_for(
        SAMPLE_PACKAGE, "remounted-echo", SAMPLE_CAPABILITY, SAMPLE_SKILL,
        SAMPLE_SERVER, "changed:",
    )]).unwrap();
    assert!(matches!(
        registry.preflight_invocation(&compiled, &active, &request),
        Err(KernelError::CapabilityProvenanceDrift { .. })
    ));
    assert!(matches!(
        registry.invoke(&compiled, &active, request).await,
        Err(KernelError::CapabilityProvenanceDrift { .. })
    ));
}

#[tokio::test]
async fn role_preflight_does_not_acquire_resources() {
    let releases = Arc::new(AtomicUsize::new(0));
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    ).unwrap();
    let materialized = registry.replace_all(vec![operation_role_registration(
        Arc::new(Mutex::new(None)), releases.clone(),
    )]).unwrap();
    let owner = principal("role-preflight-owner");
    let mut revision = sample_revision(&owner.principal_id);
    revision.payload.enabled_capabilities = vec![nomifun_agent_contracts::CapabilitySelection {
        capability: CapabilityRef {
            id: CapabilityId::from(SAMPLE_ROLE_TOOL),
        },
        action_allowlist: BTreeSet::from([ActionId::from(SAMPLE_ROLE_ACTION)]),
    }];
    revision.payload.skill_bindings.clear();
    let provider = materialized.role_provider(
        &ExecutionRoleId::from(SAMPLE_ROLE), &PluginMountId::from(SAMPLE_MOUNT),
    ).unwrap();
    revision.payload.system_role_provider_overrides.insert(
        ExecutionRoleId::from(SAMPLE_ROLE), nomifun_agent_contracts::RoleProviderSelection {
            role: provider.provider.role.clone(),
            provider_mount_id: PluginMountId::from(SAMPLE_MOUNT),
        },
    );
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let mut environment = compiler_environment(materialized.registry_digest.clone());
    environment.host_surface = "test".into();
    let compiled = AgentPresetCompiler::compile(
        &materialized, &environment, compile_request(revision, owner.clone()),
    ).unwrap().with_target_resource_bindings(
        &owner, vec![role_resource_binding(&owner.principal_id)],
    ).unwrap();
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
    let mut request = invocation(&compiled, owner, active.generation, "hello");
    request.capability_id = CapabilityId::from(SAMPLE_ROLE_TOOL);
    request.action_id = ActionId::from(SAMPLE_ROLE_ACTION);
    request.resource_binding_ids = BTreeSet::from([ResourceBindingId::from(SAMPLE_ROLE_BINDING)]);
    request.input = StrictJsonValue(json!({"value": "hello"}));
    registry.preflight_invocation(&compiled, &active, &request).unwrap();
    registry.release_resources(&request.state_scope_key).await.unwrap();
    assert_eq!(releases.load(Ordering::Acquire), 0);
    registry.invoke(&compiled, &active, request.clone()).await.unwrap();
    registry.release_resources(&request.state_scope_key).await.unwrap();
    assert_eq!(releases.load(Ordering::Acquire), 1);
}
