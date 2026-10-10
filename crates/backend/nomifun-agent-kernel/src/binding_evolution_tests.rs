use super::*;
use nomifun_agent_contracts::{
    ChatRouteCandidate, ChatRouteFeature, ChatRouteProtocol, ChatRouteRecord,
    ChatRouteRecordSchema, ChatRouteTask, ConnectionConfigRef, ModelRouteId,
    ResolvedSnapshotEnvelope, validate_bundled_contract_evolution,
    validate_session_extension_derivation,
};

fn chat_route(route: &str) -> ChatRouteRecord {
    ChatRouteRecord {
        schema: ChatRouteRecordSchema::V1,
        task: ChatRouteTask::AgentChat,
        primary: ChatRouteCandidate {
            model_route_id: ModelRouteId::from(route), model_route_revision: 1,
            provider_id: "provider".into(), model: route.into(),
            protocol: ChatRouteProtocol::OpenaiChat,
            connection_config_ref: ConnectionConfigRef::from("connection"),
            config_revision_digest: DigestHex::from("a".repeat(64)),
            credential_ref: "credential".into(),
            features: BTreeSet::from([ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput]),
            activation_features: BTreeSet::new(),
        },
        failovers: Vec::new(),
    }
}

fn set_route(revision: &mut AgentPresetRevision, route: &str) {
    let record = chat_route(route);
    revision.payload.model_route_refs.insert("agent_chat".into(), record.primary.model_route_id.clone());
    revision.payload.chat_route_records.insert("agent_chat".into(), record);
    revision.reference.revision_digest = revision.revision_digest().unwrap();
}

fn bundled_registration() -> PluginRegistration {
    bundled_without_package_skills(sample_registration(""))
}

fn bundled_without_package_skills(mut registration: PluginRegistration) -> PluginRegistration {
    registration.metadata.source.source_kind = PluginSourceKind::Bundled;
    registration.metadata.context.source.source_kind = PluginSourceKind::Bundled;
    registration.metadata.manifest.payload.contributions.skills.clear();
    registration.metadata.manifest.payload.contributions.mcp_tools.clear();
    registration.metadata.registrar.declared_skill_ids.clear();
    registration.metadata.registrar.declared_mcp_tool_keys.clear();
    refresh_manifest(&mut registration);
    registration
}

fn update_direct_locks(revision: &mut AgentPresetRevision, registry: &crate::MaterializedRegistry) {
    revision.contribution_locks = revision.payload.enabled_capabilities.iter()
        .map(|selection| registry.capability(&selection.capability.id).unwrap().contribution_lock.clone()).collect();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
}

fn revision_for(registry: &crate::MaterializedRegistry) -> AgentPresetRevision {
    let mut revision = sample_revision("binding-owner");
    revision.payload.skill_bindings.clear();
    revision.contribution_locks = vec![registry.capability(&SAMPLE_CAPABILITY.into()).unwrap().contribution_lock.clone()];
    set_route(&mut revision, "first-model");
    revision
}

fn refresh_snapshot(snapshot: &mut ResolvedSnapshotEnvelope) {
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
}

#[test]
fn model_derivation_preserves_frozen_contract_and_matches_original_compiler() {
    let registry = Materializer::materialize(&MaterializationPolicy::stable(VERSION), &[bundled_registration()], 1).unwrap();
    let source_revision = revision_for(&registry);
    let owner = principal("binding-owner");
    let environment = compiler_environment(registry.registry_digest.clone());
    let source = AgentPresetCompiler::compile(&registry, &environment,
        compile_request(source_revision.clone(), owner.clone())).unwrap();
    let mut target_revision = source_revision.clone();
    target_revision.reference.preset_id = "model-variant".into();
    set_route(&mut target_revision, "second-model");
    let request = compile_request(target_revision.clone(), owner.clone());
    let derived = AgentPresetCompiler::derive_model_snapshot(&source_revision, &source.envelope, request.clone()).unwrap();
    let original_compile = AgentPresetCompiler::compile(&registry, &environment, request).unwrap();
    assert_eq!(derived, original_compile.envelope);
    assert_eq!(derived.content.enabled_capabilities, source.content().enabled_capabilities);
    assert!(validate_bundled_contract_evolution(&source_revision, &source.envelope,
        &target_revision, &derived).is_err(), "forward capability evolution must not silently change a model");

    let mut repeated_revision = target_revision.clone();
    repeated_revision.reference.preset_id = "second-model-variant".into();
    set_route(&mut repeated_revision, "third-model");
    let repeated = AgentPresetCompiler::derive_model_snapshot(&target_revision, &derived,
        compile_request(repeated_revision, owner.clone())).unwrap();
    assert_eq!(repeated.content.enabled_capabilities, source.content().enabled_capabilities);

    for case in 0..4 {
        let mut invalid = target_revision.clone();
        match case {
            0 => invalid.payload.instructions.push_str("new instructions"),
            1 => invalid.payload.enabled_capabilities[0].action_allowlist.clear(),
            2 => invalid.contribution_locks[0].contract_digest = "c".repeat(64).into(),
            3 => { invalid.payload.model_route_refs.insert("image_generation".into(), "another-route".into()); },
            _ => unreachable!(),
        }
        invalid.reference.revision_digest = invalid.revision_digest().unwrap();
        assert!(AgentPresetCompiler::derive_model_snapshot(&source_revision, &source.envelope,
            compile_request(invalid, owner.clone())).is_err(), "case {case}");
    }
}

#[test]
fn bundled_evolution_accepts_forward_schemas_but_rejects_authority_and_source_drift() {
    let registry = Materializer::materialize(&MaterializationPolicy::stable(VERSION), &[bundled_registration()], 1).unwrap();
    let revision = revision_for(&registry);
    let source = AgentPresetCompiler::compile(&registry, &compiler_environment(registry.registry_digest.clone()),
        compile_request(revision.clone(), principal("binding-owner"))).unwrap().envelope;
    let mut target_revision = revision.clone();
    target_revision.reference.preset_id = "evolved-binding".into();
    target_revision.contribution_locks[0].contract_digest = "c".repeat(64).into();
    target_revision.reference.revision_digest = target_revision.revision_digest().unwrap();
    let mut target = source.clone();
    target.content.preset_revision_ref = target_revision.reference.clone();
    target.content.chat_route_identity = target_revision.chat_route_identity().unwrap();
    let capability = &mut target.content.enabled_capabilities[0];
    capability.schema_digest = "c".repeat(64).into();
    capability.contribution_lock = target_revision.contribution_locks[0].clone();
    capability.target_artifact_digest = "d".repeat(64).into();
    capability.description = Some("New publisher description".into());
    capability.actions[0].input_schema = "schema://new-input".into();
    refresh_snapshot(&mut target);
    validate_bundled_contract_evolution(&revision, &source, &target_revision, &target).unwrap();
    for case in 0..9 {
        let mut invalid = target.clone();
        let capability = &mut invalid.content.enabled_capabilities[0];
        match case {
            0 => capability.resolved_source.source_kind = PluginSourceKind::ManagedLocal,
            1 => capability.resolved_source.source_identity = "another-publisher".into(),
            2 => capability.actions[0].effect_class = EffectClass::ExternalTransmit,
            3 => { capability.required_resource_kinds.insert("another-resource".into()); },
            4 => { capability.resolved_mount_id = Some("another-mount".into()); },
            5 => capability.source_package.version = "2.0.0".into(),
            6 => { capability.required_runtime_features.insert("new-feature".into()); },
            7 => {
                let mut action = capability.actions[0].clone();
                action.action_id = "another-action".into();
                capability.actions.push(action);
            },
            8 => { capability.contribution_lock.source_identity = "another-lock-owner".into(); },
            _ => unreachable!(),
        };
        refresh_snapshot(&mut invalid);
        assert!(validate_bundled_contract_evolution(&revision, &source, &target_revision, &invalid).is_err(), "case {case}");
    }
}

#[test]
fn frozen_provider_compilation_uses_saved_mount_and_rejects_role_generation_change() {
    let registration = operation_role_registration(Arc::new(Mutex::new(None)), Arc::new(AtomicUsize::new(0)));
    let registry = Materializer::materialize(&MaterializationPolicy::stable_with_test_fixtures(VERSION), &[registration], 1).unwrap();
    let contract = registry.role_contract(&SAMPLE_ROLE.into()).unwrap();
    let mut revision = sample_revision("binding-owner");
    revision.payload.skill_bindings.clear();
    revision.payload.enabled_capabilities = vec![nomifun_agent_contracts::CapabilitySelection {
        capability: CapabilityRef { id: SAMPLE_ROLE_TOOL.into() }, action_allowlist: BTreeSet::from([SAMPLE_ROLE_ACTION.into()]),
    }];
    revision.payload.system_role_provider_overrides.insert(SAMPLE_ROLE.into(), nomifun_agent_contracts::RoleProviderSelection {
        role: ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() },
        provider_mount_id: SAMPLE_MOUNT.into(),
    });
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let mut environment = compiler_environment(registry.registry_digest.clone());
    environment.host_surface = "test".into();
    let request = compile_request(revision.clone(), principal("binding-owner"));
    let source = AgentPresetCompiler::compile(&registry, &environment, request.clone()).unwrap();
    let mut environment_with_wrong_default = environment.clone();
    environment_with_wrong_default.installation_role_bindings.insert(SAMPLE_ROLE.into(), nomifun_agent_contracts::InstallationRoleBinding {
        selection: nomifun_agent_contracts::RoleProviderSelection {
            role: ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() },
            provider_mount_id: "wrong-default".into(),
        },
        binding_version: 2, updated_at_ms: 2,
    });
    let compiled = AgentPresetCompiler::compile_with_frozen_role_providers(&registry, &environment_with_wrong_default,
        request.clone(), &source.envelope).unwrap();
    assert_eq!(compiled.envelope, source.envelope);

    // A previous artifact has different exact pins on the same saved mount.
    // Rebinding refreshes resolution only; the author's configuration remains
    // the old immutable payload, and an ordinary authoring compile rejects it.
    let mut previous_revision = revision;
    previous_revision.payload.system_role_provider_overrides.get_mut(&SAMPLE_ROLE.into()).unwrap()
        .role.contract_digest = "f".repeat(64).into();
    previous_revision.reference.revision_digest = previous_revision.revision_digest().unwrap();
    let mut previous = source.envelope.clone();
    previous.content.preset_revision_ref = previous_revision.reference.clone();
    previous.content.resolved_role_providers.get_mut(&SAMPLE_ROLE.into()).unwrap()
        .provider.role.contract_digest = "f".repeat(64).into();
    refresh_snapshot(&mut previous);
    let previous_request = compile_request(previous_revision.clone(), principal("binding-owner"));
    assert!(AgentPresetCompiler::compile(&registry, &environment_with_wrong_default, previous_request.clone()).is_err());
    let current = AgentPresetCompiler::compile_with_frozen_role_providers(&registry, &environment_with_wrong_default,
        previous_request, &previous).unwrap();
    assert_eq!(current.content().preset_revision_ref, previous_revision.reference);
    assert_eq!(current.content().resolved_role_providers, source.content().resolved_role_providers);
    let mut invalid = source.envelope.clone();
    invalid.content.resolved_role_providers.get_mut(&SAMPLE_ROLE.into()).unwrap().provider.role.key.contract_version = "2.0.0".into();
    refresh_snapshot(&mut invalid);
    assert!(AgentPresetCompiler::compile_with_frozen_role_providers(&registry, &environment_with_wrong_default,
        request, &invalid).is_err());
}

#[test]
fn explicit_new_role_selection_keeps_normal_resolution_without_replacing_frozen_roles() {
    let registration = operation_role_registration(Arc::new(Mutex::new(None)), Arc::new(AtomicUsize::new(0)));
    let registry = Materializer::materialize(&MaterializationPolicy::stable_with_test_fixtures(VERSION), &[registration], 1).unwrap();
    let contract = registry.role_contract(&SAMPLE_ROLE.into()).unwrap();
    let selection = nomifun_agent_contracts::RoleProviderSelection {
        role: ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() },
        provider_mount_id: SAMPLE_MOUNT.into(),
    };
    let mut environment = compiler_environment(registry.registry_digest.clone());
    environment.host_surface = "test".into();
    environment.installation_role_bindings.insert(SAMPLE_ROLE.into(), nomifun_agent_contracts::InstallationRoleBinding {
        selection: selection.clone(), binding_version: 1, updated_at_ms: 1,
    });
    let mut source_revision = sample_revision("binding-owner");
    source_revision.payload.skill_bindings.clear();
    source_revision.reference.revision_digest = source_revision.revision_digest().unwrap();
    let source = AgentPresetCompiler::compile(&registry, &environment,
        compile_request(source_revision.clone(), principal("binding-owner"))).unwrap();
    assert!(source.content().resolved_role_providers.is_empty());
    for explicit in [false, true] {
        let mut selected_revision = source_revision.clone();
        selected_revision.payload.enabled_capabilities.push(nomifun_agent_contracts::CapabilitySelection {
            capability: CapabilityRef { id: SAMPLE_ROLE_TOOL.into() },
            action_allowlist: BTreeSet::from([SAMPLE_ROLE_ACTION.into()]),
        });
        if explicit {
            selected_revision.payload.system_role_provider_overrides.insert(SAMPLE_ROLE.into(), selection.clone());
            environment.installation_role_bindings.clear();
        }
        selected_revision.reference.revision_digest = selected_revision.revision_digest().unwrap();
        let selected = AgentPresetCompiler::compile_with_frozen_role_providers(&registry, &environment,
            compile_request(selected_revision.clone(), principal("binding-owner")), &source.envelope).unwrap();
        assert_eq!(selected.content().resolved_role_providers[&SAMPLE_ROLE.into()].provider.mount_id, AgentModuleId::from(SAMPLE_MOUNT));
        assert!(validate_bundled_contract_evolution(&source_revision, &source.envelope,
            &selected_revision, &selected.envelope).is_err(), "only explicit selection authority may introduce the new role");
    }
}

#[test]
fn removing_an_extension_does_not_resolve_its_retired_role() {
    let registration = operation_role_registration(Arc::new(Mutex::new(None)), Arc::new(AtomicUsize::new(0)));
    let registry = Materializer::materialize(&MaterializationPolicy::stable_with_test_fixtures(VERSION), &[registration], 1).unwrap();
    let contract = registry.role_contract(&SAMPLE_ROLE.into()).unwrap();
    let mut revision = sample_revision("binding-owner");
    revision.payload.skill_bindings.clear();
    revision.payload.enabled_capabilities.push(nomifun_agent_contracts::CapabilitySelection {
        capability: CapabilityRef { id: SAMPLE_ROLE_TOOL.into() },
        action_allowlist: BTreeSet::from([SAMPLE_ROLE_ACTION.into()]),
    });
    revision.payload.system_role_provider_overrides.insert(SAMPLE_ROLE.into(), nomifun_agent_contracts::RoleProviderSelection {
        role: ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() },
        provider_mount_id: SAMPLE_MOUNT.into(),
    });
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let mut environment = compiler_environment(registry.registry_digest.clone());
    environment.host_surface = "test".into();
    let source = AgentPresetCompiler::compile(&registry, &environment,
        compile_request(revision.clone(), principal("binding-owner"))).unwrap();
    assert!(!source.content().resolved_role_providers.is_empty());

    // The new installation no longer publishes any part of the removed Role.
    // Only the selected roots determine which saved pins must resolve.
    let current = Materializer::materialize(&MaterializationPolicy::stable_with_test_fixtures(VERSION),
        &[sample_registration("")], 2).unwrap();
    assert!(current.role_contract(&SAMPLE_ROLE.into()).is_none());
    assert!(current.capability(&SAMPLE_ROLE_TOOL.into()).is_none());
    environment.target_contribution_manifest_digest = current.registry_digest.clone();
    let mut removed_revision = revision.clone();
    removed_revision.payload.enabled_capabilities.retain(|selection| selection.capability.id.as_ref() != SAMPLE_ROLE_TOOL);
    removed_revision.reference.revision_digest = removed_revision.revision_digest().unwrap();
    let authorial_overrides = removed_revision.payload.system_role_provider_overrides.clone();
    let removed = AgentPresetCompiler::compile_with_frozen_role_providers(&current, &environment,
        compile_request(removed_revision.clone(), principal("binding-owner")), &source.envelope).unwrap();
    assert!(removed.content().resolved_role_providers.is_empty());
    assert_eq!(removed_revision.payload.system_role_provider_overrides, authorial_overrides);
    assert!(AgentPresetCompiler::compile_with_frozen_role_providers(&current, &environment,
        compile_request(revision, principal("binding-owner")), &source.envelope).is_err(),
        "a still-selected retired Role must fail rather than use a default or another provider");
}

#[test]
fn explicit_extension_derivation_protects_retained_infrastructure_and_global_fields() {
    let policy = MaterializationPolicy::stable(VERSION);
    let registry = Materializer::materialize(&policy, &[bundled_registration()], 1).unwrap();
    let revision = revision_for(&registry);
    let source = AgentPresetCompiler::compile(&registry, &compiler_environment(registry.registry_digest.clone()),
        compile_request(revision.clone(), principal("binding-owner"))).unwrap();
    let roots = BTreeSet::from([CapabilityId::from(SAMPLE_CAPABILITY)]);
    for case in 0..7 {
        let mut registration = bundled_registration();
        let manifest = &mut registration.metadata.manifest.payload.contributions.capabilities[0];
        manifest.display.description = "Current publisher description".into();
        manifest.contributions.actions[0].input_schema = format!("schema://{SAMPLE_CAPABILITY}/input@1#{}",
            digest_payload(&json!({"type":"object", "description":"Current input schema"})).unwrap().as_ref()).into();
        match case {
            1 => manifest.contributions.actions[0].effect_class = EffectClass::ExternalTransmit,
            2 => { manifest.contributions.resource_kinds.insert("additional-core-resource".into()); },
            3 => {
                let mut action = manifest.contributions.actions[0].clone();
                action.action_id = "additional-core-action".into();
                manifest.contributions.actions.push(action);
            },
            4 => {
                registration.metadata.source.source_identity = "different-publisher".into();
                registration.metadata.context.source.source_identity = "different-publisher".into();
            },
            _ => {},
        }
        refresh_manifest(&mut registration);
        let current = Materializer::materialize(&policy, &[registration], 2).unwrap();
        let mut candidate_revision = revision.clone();
        candidate_revision.reference.preset_id = "extension-selection".into();
        candidate_revision.payload.skill_bindings.push(nomifun_agent_contracts::AgentSkillBinding::library_selected(
            nomifun_agent_contracts::FrozenLibrarySkill::new("Selected guidance".into(), "Selected by the Session owner".into(),
                nomifun_agent_contracts::LibrarySkillSource::Custom, "Use the current schema.".into(), BTreeMap::new()).unwrap(), true));
        match case {
            5 => candidate_revision.payload.instructions.push_str(" and replace core behavior"),
            6 => set_route(&mut candidate_revision, "different-model"),
            _ => {},
        }
        update_direct_locks(&mut candidate_revision, &current);
        let candidate = AgentPresetCompiler::compile(&current, &compiler_environment(current.registry_digest.clone()),
            compile_request(candidate_revision.clone(), principal("binding-owner"))).unwrap();
        let result = validate_session_extension_derivation(&revision, &source.envelope,
            &candidate_revision, &candidate.envelope, &roots, &roots);
        if case == 0 {
            result.unwrap();
            assert_ne!(source.content().skill_locks, candidate.content().skill_locks);
            assert_ne!(source.content().enabled_capabilities[0].schema_digest, candidate.content().enabled_capabilities[0].schema_digest);
        } else {
            assert!(result.is_err(), "case {case}: explicitly mutable infrastructure remains protected");
        }
    }
}

#[test]
fn explicit_extension_derivation_protects_mutable_roots_reachable_from_core() {
    const EXTENSION: &str = "sample.extension.tool";
    let registrations = || {
        let mut core = bundled_registration();
        core.metadata.manifest.payload.contributions.capabilities[0].requires.push(CapabilityRef { id: EXTENSION.into() });
        refresh_manifest(&mut core);
        let extension = bundled_without_package_skills(registration_for("sample.extension.package", "sample.extension.mount",
            EXTENSION, "sample.extension.skill", "sample.extension.server", ""));
        [core, extension]
    };
    let policy = MaterializationPolicy::stable(VERSION);
    let registry = Materializer::materialize(&policy, &registrations(), 1).unwrap();
    let mut revision = revision_for(&registry);
    revision.payload.enabled_capabilities.push(nomifun_agent_contracts::CapabilitySelection {
        capability: CapabilityRef { id: EXTENSION.into() }, action_allowlist: BTreeSet::from([SAMPLE_ACTION.into()]),
    });
    update_direct_locks(&mut revision, &registry);
    let source = AgentPresetCompiler::compile(&registry, &compiler_environment(registry.registry_digest.clone()),
        compile_request(revision.clone(), principal("binding-owner"))).unwrap();
    let mutable = BTreeSet::from([CapabilityId::from(EXTENSION)]);
    for changed_effect in [false, true] {
        let mut current_registrations = registrations();
        let extension = &mut current_registrations[1].metadata.manifest.payload.contributions.capabilities[0];
        extension.contributions.actions[0].input_schema = format!("schema://{EXTENSION}/input@1#{}",
            digest_payload(&json!({"type":"object", "description":"Forward schema"})).unwrap().as_ref()).into();
        if changed_effect {
            extension.contributions.actions[0].effect_class = EffectClass::ExternalTransmit;
        }
        refresh_manifest(&mut current_registrations[1]);
        let current = Materializer::materialize(&policy, &current_registrations, 2).unwrap();
        let mut candidate_revision = revision.clone();
        candidate_revision.reference.preset_id = "shared-extension-selection".into();
        update_direct_locks(&mut candidate_revision, &current);
        let candidate = AgentPresetCompiler::compile(&current, &compiler_environment(current.registry_digest.clone()),
            compile_request(candidate_revision.clone(), principal("binding-owner"))).unwrap();
        let result = validate_session_extension_derivation(&revision, &source.envelope,
            &candidate_revision, &candidate.envelope, &mutable, &BTreeSet::new());
        if changed_effect {
            assert!(result.is_err(), "an explicitly mutable root remains frozen when reachable from an immutable core root");
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn explicit_extension_derivation_rejects_a_new_generation_of_a_protected_role() {
    let registration = || bundled_without_package_skills(operation_role_registration(
        Arc::new(Mutex::new(None)), Arc::new(AtomicUsize::new(0))));
    let policy = MaterializationPolicy::stable(VERSION);
    let registry = Materializer::materialize(&policy, &[registration()], 1).unwrap();
    let mut revision = revision_for(&registry);
    revision.payload.enabled_capabilities.push(nomifun_agent_contracts::CapabilitySelection {
        capability: CapabilityRef { id: SAMPLE_ROLE_TOOL.into() }, action_allowlist: BTreeSet::from([SAMPLE_ROLE_ACTION.into()]),
    });
    update_direct_locks(&mut revision, &registry);
    let environment_for = |registry: &crate::MaterializedRegistry| {
        let mut environment = compiler_environment(registry.registry_digest.clone());
        environment.host_surface = "test".into();
        let contract = registry.role_contract(&SAMPLE_ROLE.into()).unwrap();
        environment.installation_role_bindings.insert(SAMPLE_ROLE.into(), nomifun_agent_contracts::InstallationRoleBinding {
            selection: nomifun_agent_contracts::RoleProviderSelection {
                role: ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() },
                provider_mount_id: SAMPLE_MOUNT.into(),
            }, binding_version: 1, updated_at_ms: 1,
        });
        environment
    };
    let source = AgentPresetCompiler::compile(&registry, &environment_for(&registry),
        compile_request(revision.clone(), principal("binding-owner"))).unwrap();
    let mut current_registration = registration();
    let contributions = &mut current_registration.metadata.manifest.payload.contributions;
    contributions.role_contracts[0].key.contract_version = "2.0.0".into();
    contributions.role_providers[0].role = ExactRoleContractRef {
        key: contributions.role_contracts[0].key.clone(), contract_digest: digest_payload(&contributions.role_contracts[0]).unwrap(),
    };
    refresh_manifest(&mut current_registration);
    let current = Materializer::materialize(&policy, &[current_registration], 2).unwrap();
    let mut candidate_revision = revision.clone();
    candidate_revision.reference.preset_id = "changed-role-selection".into();
    update_direct_locks(&mut candidate_revision, &current);
    let candidate = AgentPresetCompiler::compile(&current, &environment_for(&current),
        compile_request(candidate_revision.clone(), principal("binding-owner"))).unwrap();
    assert!(validate_session_extension_derivation(&revision, &source.envelope, &candidate_revision,
        &candidate.envelope, &BTreeSet::from([CapabilityId::from(SAMPLE_CAPABILITY)]), &BTreeSet::new()).is_err(),
        "a changed installation default must not replace an immutable core Role generation");
}
