use super::*;
use nomifun_agent_kernel::{
    AgentPresetCompiler, CompileRequest, CompilerEnvironment, MaterializedRegistry,
};
use nomifun_ai_agent::model_middleware;

fn digest(fill: char) -> DigestHex {
    fill.to_string().repeat(64).into()
}

struct FixtureAdapter;

#[async_trait::async_trait]
impl nomifun_agent_kernel::CapabilityHandler for FixtureAdapter {
    async fn invoke(
        &self,
        _context: nomifun_agent_kernel::CapabilityInvocationContext,
        _input: StrictJsonValue,
    ) -> Result<StrictJsonValue, nomifun_agent_kernel::KernelError> {
        Ok(StrictJsonValue(serde_json::json!({})))
    }
}

#[async_trait::async_trait]
impl nomifun_agent_kernel::CapabilityContextContributionFactory for FixtureAdapter {
    async fn contribute(
        &self,
        _request: nomifun_agent_kernel::CapabilityContextContributionRequest,
    ) -> Result<nomifun_agent_kernel::ContextContributionResult, nomifun_agent_kernel::KernelError>
    {
        Ok(nomifun_agent_kernel::ContextContributionResult { value: None })
    }
}

fn product() -> ResolvedCapability {
    let contribution_id = ContributionId::from("capability:fixture.middleware");
    let action = model_middleware::action();
    ResolvedCapability {
        consumption: Default::default(),
        dependency_refs: Vec::new(),
        capability: CapabilityRef {
            id: "fixture.middleware".into(),
            version: "1.0.0".into(),
        },
        source_package: PackageRef {
            id: "fixture".into(),
            version: "1.0.0".into(),
        },
        contribution_id: contribution_id.clone(),
        contribution_lock: ContributionLock {
            source_kind: ContributionSourceKind::PluginProductActiveRelease,
            source_identity: "plugin-product:fixture".into(),
            mount_id: None,
            plugin_product_id: Some("fixture".into()),
            mcp_binding_id: None,
            contribution_id,
            contract_digest: digest('a'),
        },
        resolved_mount_id: None,
        resolved_source: PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: "plugin-product:fixture".into(),
            source_digest: Some(digest('b')),
        },
        target_artifact_digest: digest('b'),
        schema_digest: digest('a'),
        dependency_path: vec!["fixture.middleware".into()],
        required_runtime_features: BTreeSet::new(),
        plugin_product_id: Some("fixture".into()),
        active_release: Some(PluginReleaseRef {
            release_id: "fixture-release".into(),
            artifact_id: "fixture-artifact".into(),
            release_digest: digest('b'),
            manifest_digest: digest('c'),
        }),
        active_release_epoch: Some(1),
        catalog_digest: Some(digest('d')),
        display_name: Some("Fixture middleware".into()),
        description: Some("Consumer boundary fixture".into()),
        actions: vec![action.clone()],
        required_resource_kinds: BTreeSet::new(),
        action_allowlist: BTreeSet::from([action.action_id]),
    }
}

fn compile(capability: ResolvedCapability, ordered: bool) -> ResolvedSnapshotEnvelope {
    compile_selection(
        &MaterializedRegistry::empty(), capability.capability.clone(),
        capability.action_allowlist.clone(), capability.contribution_lock.clone(),
        vec![capability], ordered,
    )
}

fn compile_selection(
    registry: &MaterializedRegistry,
    capability: CapabilityRef,
    action_allowlist: BTreeSet<ActionId>,
    contribution_lock: ContributionLock,
    product_capabilities: Vec<ResolvedCapability>,
    ordered: bool,
) -> ResolvedSnapshotEnvelope {
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: "fixture-preset".into(),
            revision: 1,
            revision_digest: digest('0'),
        },
        payload: AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: if ordered {
                vec![capability.id.clone()]
            } else {
                Vec::new()
            },
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: capability.clone(),
                action_allowlist: action_allowlist.clone(),
            }],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "fixture".into(),
            instructions: "fixture".into(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        },
        contribution_locks: vec![contribution_lock],
        created_by: "fixture-owner".into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    AgentPresetCompiler::compile(
        registry,
        &CompilerEnvironment {
            resolver_version: "1.0.0".into(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: digest('e'),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: digest('f'),
            target_contribution_manifest_digest: registry.registry_digest.clone(),
            host_target: "windows-desktop-x64".into(),
            host_surface: "desktop".into(),
            availability_evidence_revision: "middleware-boundary-test".into(),
        },
        CompileRequest {
            revision,
            plugin_product_capabilities: product_capabilities,
            principal: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: "fixture-owner".into(),
            },
            scene: "test".into(),
            surface: "desktop".into(),
            audience: "test".into(),
            created_at_ms: 2,
            resolver_run_id: "middleware-boundary-test".into(),
        },
    )
    .expect("structurally valid registered or Product selection should compile")
    .envelope
}

fn assert_nomi_rejects(snapshot: &ResolvedSnapshotEnvelope) {
    // Exercise both the runtime-shared consumer and the actual preview/save gate.
    assert!(model_middleware::validate_selection(&snapshot.content).is_err());
    assert!(validate_snapshot(&MaterializedRegistry::empty(), snapshot).is_err());
}

fn mount_snapshot(
    action: Option<CapabilityActionDescriptor>,
    source_kind: PluginSourceKind,
    ordered: bool,
) -> (Arc<MaterializedRegistry>, ResolvedSnapshotEnvelope) {
    use nomifun_agent_kernel::{InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy};
    const ID: &str = "fixture.mount-middleware";
    const CAPABILITIES: &[CapabilitySpec] = &[CapabilitySpec::middleware(ID)];
    let mut registration = nomifun_agent_domain_support::registration(PackageSpec {
        id: "fixture.mount", mount_id: "fixture-mount", display_name: "Mount middleware",
        description: "Registered source admission fixture", capabilities: CAPABILITIES,
        supported_surfaces: &["desktop"],
    }).unwrap();
    registration.metadata.source.source_kind = source_kind;
    registration.metadata.context.source.source_kind = source_kind;
    let manifest = &mut registration.metadata.manifest.payload.contributions.capabilities[0];
    let has_action = action.is_some();
    // CapabilityKind is only a catalog summary. Direct authoring and ordering
    // follow the actual Action/Context contribution set.
    manifest.kind = if has_action {
        CapabilityKind::Tool
    } else {
        CapabilityKind::ContextContributor
    };
    let action_allowlist = action
        .as_ref()
        .map(|action| BTreeSet::from([action.action_id.clone()]))
        .unwrap_or_default();
    if let Some(action) = action {
        manifest.contributions.actions = vec![action];
    } else {
        manifest.contributions.context_schema_refs = vec![CanonicalSchemaRef::from(format!(
            "schema://fixture.mount/context@1#{}", digest_payload(&serde_json::json!({"type":"object"})).unwrap().as_ref(),
        ))];
    }
    registration.metadata.manifest = ArtifactEnvelope::new(registration.metadata.manifest.payload).unwrap();
    if has_action {
        registration
            .add_capability_handler(ID.into(), Arc::new(FixtureAdapter))
            .unwrap();
    } else {
        registration
            .add_capability_context_factory(ID.into(), Arc::new(FixtureAdapter))
            .unwrap();
    }
    let mut policy = MaterializationPolicy::stable("1.0.0");
    policy.allowed_sources.insert(PluginSourceKind::ManagedLocal);
    let registry = KernelRegistry::new(policy, Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
    let materialized = registry.replace_all(vec![registration]).unwrap();
    let registered = materialized.capability(&ID.into()).unwrap();
    let snapshot = compile_selection(
        &materialized,
        CapabilityRef { id: registered.manifest.id.clone(), version: registered.manifest.version.clone() },
        action_allowlist, registered.contribution_lock.clone(), Vec::new(), ordered,
    );
    snapshot.validate().unwrap();
    let resolved = &snapshot.content.enabled_capabilities[0];
    resolved.validate().unwrap();
    assert_eq!(
        resolved.actions,
        registered.manifest.contributions.actions,
        "Generic compilation must freeze the exact materialized Action contract",
    );
    assert_eq!(resolved.contribution_lock.source_kind,
        if source_kind == PluginSourceKind::ManagedLocal { ContributionSourceKind::PluginMount }
        else { ContributionSourceKind::PlatformBuiltin });
    (materialized, snapshot)
}

#[test]
fn registered_mount_and_bundled_hooks_are_rejected_even_without_middleware_order() {
    for source in [PluginSourceKind::ManagedLocal, PluginSourceKind::Bundled] {
        for action in [nomifun_ai_agent::tool_middleware::before_action(), model_middleware::action()] {
            for ordered in [false, true] {
                let (registry, snapshot) = mount_snapshot(Some(action.clone()), source, ordered);
                if !ordered {
                    assert!(snapshot.content.middleware_order.is_empty());
                    // Middleware ordering is optional, but the frozen Action
                    // contract remains visible and must still fail closed at
                    // the consumer boundary for non-Product sources.
                    assert!(
                        nomifun_ai_agent::tool_middleware::validate_selection(&snapshot.content)
                            .is_err()
                            || model_middleware::validate_selection(&snapshot.content).is_err()
                    );
                }
                let error = validate_snapshot(&registry, &snapshot).unwrap_err();
                assert!(error.to_string().contains("Plugin Product Active Release"), "{error}");
                assert!(error.to_string().contains(action.action_id.as_ref()), "{error}");
            }
        }
    }
}

#[test]
fn registered_context_middleware_is_not_rejected_as_a_product_hook() {
    let (registry, snapshot) = mount_snapshot(None, PluginSourceKind::ManagedLocal, false);
    validate_snapshot(&registry, &snapshot).unwrap();
}

#[test]
fn before_tool_consumer_rejects_product_descriptor_drift() {
    for mismatch in ["effect", "schema", "presentation", "extra_action"] {
        let mut item = product();
        item.actions = vec![nomifun_ai_agent::tool_middleware::before_action()];
        item.action_allowlist = BTreeSet::from([
            nomifun_ai_agent::tool_middleware::BEFORE_ACTION_ID.into(),
        ]);
        match mismatch {
            "effect" => item.actions[0].effect_class = EffectClass::ExecuteLocal,
            "schema" => item.actions[0].input_schema = "schema://fixture/other".into(),
            "presentation" => item.actions[0].presentation = ToolPresentationKind::FunctionTool,
            _ => { let mut extra = item.actions[0].clone(); extra.action_id = "fixture.extra".into(); item.actions.push(extra); }
        }
        let snapshot = compile(item, false);
        assert!(nomifun_ai_agent::tool_middleware::validate_selection(&snapshot.content).is_err());
        assert!(validate_snapshot(&MaterializedRegistry::empty(), &snapshot).is_err());
    }
}

#[test]
fn middleware_boundary_generic_compile_accepts_alternative_but_nomi_rejects() {
    let mut alternative = product();
    alternative.actions[0].action_id = "agent.after_model".into();
    alternative.actions[0].input_schema = "schema://fixture/after-model/input".into();
    alternative.actions[0].output_schema = "schema://fixture/after-model/output".into();
    alternative.action_allowlist = BTreeSet::from(["agent.after_model".into()]);
    let snapshot = compile(alternative.clone(), true);
    snapshot.validate().unwrap();
    assert_eq!(snapshot.content.enabled_capabilities, vec![alternative]);
    assert_nomi_rejects(&snapshot);
}

#[test]
fn middleware_boundary_nomi_accepts_exact_contract_with_optional_order() {
    for ordered in [false, true] {
        let snapshot = compile(product(), ordered);
        model_middleware::validate_selection(&snapshot.content).unwrap();
        validate_snapshot(&MaterializedRegistry::empty(), &snapshot).unwrap();
    }
}

#[test]
fn middleware_boundary_nomi_retains_exact_descriptor_and_resource_checks() {
    for ordered in [false, true] {
        for mismatch in [
            "input",
            "output",
            "effect",
            "presentation",
            "extra_action",
            "resource",
        ] {
            let mut capability = product();
            match mismatch {
                "input" => capability.actions[0].input_schema = "schema://fixture/input".into(),
                "output" => capability.actions[0].output_schema = "schema://fixture/output".into(),
                "effect" => capability.actions[0].effect_class = EffectClass::WriteDurable,
                "presentation" => {
                    capability.actions[0].presentation = ToolPresentationKind::FunctionTool
                }
                "extra_action" => {
                    let mut extra = model_middleware::action();
                    extra.action_id = "fixture.extra".into();
                    capability.actions.push(extra);
                }
                "resource" => {
                    capability
                        .required_resource_kinds
                        .insert("workspace".into());
                }
                _ => unreachable!(),
            }
            let snapshot = compile(capability, ordered);
            assert_nomi_rejects(&snapshot);
        }
    }
}

#[test]
fn middleware_boundary_nomi_retains_identity_authority_and_order_checks() {
    let snapshot = compile(product(), true);
    for mismatch in [
        "provenance",
        "contract_digest",
        "authority",
        "duplicate",
        "unselected",
        "dependency",
    ] {
        let mut invalid = snapshot.clone();
        let capability = &mut invalid.content.enabled_capabilities[0];
        match mismatch {
            "provenance" => {
                capability.contribution_lock.source_kind = ContributionSourceKind::PlatformBuiltin
            }
            "contract_digest" => capability.contribution_lock.contract_digest = digest('e'),
            "authority" => capability.action_allowlist = BTreeSet::from(["fixture.other".into()]),
            "duplicate" => invalid
                .content
                .middleware_order
                .push(capability.capability.id.clone()),
            "unselected" => invalid.content.middleware_order = vec!["fixture.unselected".into()],
            "dependency" => capability.consumption = CapabilityConsumption::Dependency,
            _ => unreachable!(),
        }
        assert_nomi_rejects(&invalid);
    }
}
