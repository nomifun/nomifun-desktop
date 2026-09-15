use super::*;
use nomifun_agent_kernel::{
    AgentPresetCompiler, CompileRequest, CompilerEnvironment, MaterializedRegistry,
};
use nomifun_ai_agent::model_middleware;

fn digest(fill: char) -> DigestHex {
    fill.to_string().repeat(64).into()
}

fn product() -> ResolvedCapability {
    let contribution_id = ContributionId::from("capability:fixture.middleware");
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
        actions: vec![model_middleware::action()],
        required_resource_kinds: BTreeSet::new(),
        action_allowlist: BTreeSet::new(),
    }
}

fn compile(capability: ResolvedCapability, ordered: bool) -> ResolvedSnapshotEnvelope {
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: "fixture-preset".into(),
            revision: 1,
            revision_digest: digest('0'),
        },
        payload: AgentPresetRevisionPayload {
            runtime_engine: None,
            context_order: Vec::new(),
            middleware_order: if ordered {
                vec![capability.capability.id.clone()]
            } else {
                Vec::new()
            },
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: capability.capability.clone(),
                action_allowlist: capability.action_allowlist.clone(),
            }],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "fixture".into(),
            instructions: "fixture".into(),
            starter_prompts: Vec::new(),
        },
        contribution_locks: vec![capability.contribution_lock.clone()],
        created_by: "fixture-owner".into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    AgentPresetCompiler::compile(
        &MaterializedRegistry::empty(),
        &CompilerEnvironment {
            resolver_version: "1.0.0".into(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: digest('e'),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: digest('f'),
            target_contribution_manifest_digest: digest('1'),
            host_target: "windows-desktop-x64".into(),
            host_surface: "desktop".into(),
            availability_evidence_revision: "middleware-boundary-test".into(),
        },
        CompileRequest {
            revision,
            plugin_product_capabilities: vec![capability],
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
    .expect("structurally valid Product projection should compile")
    .envelope
}

fn assert_nomi_rejects(snapshot: &ResolvedSnapshotEnvelope) {
    // Exercise both the runtime-shared consumer and the actual preview/save gate.
    assert!(model_middleware::validate_selection(&snapshot.content).is_err());
    assert!(validate_snapshot(&MaterializedRegistry::empty(), snapshot).is_err());
}

#[test]
fn middleware_boundary_generic_compile_accepts_alternative_but_nomi_rejects() {
    let mut alternative = product();
    alternative.actions[0].action_id = "agent.after_model".into();
    alternative.actions[0].input_schema = "schema://fixture/after-model/input".into();
    alternative.actions[0].output_schema = "schema://fixture/after-model/output".into();
    let snapshot = compile(alternative.clone(), true);
    snapshot.validate().unwrap();
    assert_eq!(snapshot.content.enabled_capabilities, vec![alternative]);
    assert_nomi_rejects(&snapshot);
}

#[test]
fn middleware_boundary_nomi_accepts_exact_contract_with_optional_order_and_allowlist() {
    for ordered in [false, true] {
        for explicit_authority in [false, true] {
            let mut capability = product();
            if explicit_authority {
                capability
                    .action_allowlist
                    .insert(model_middleware::ACTION_ID.into());
            }
            let snapshot = compile(capability, ordered);
            model_middleware::validate_selection(&snapshot.content).unwrap();
            validate_snapshot(&MaterializedRegistry::empty(), &snapshot).unwrap();
        }
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
