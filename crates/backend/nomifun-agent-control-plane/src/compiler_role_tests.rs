//! Authoring reuse checks. Execution/Node dispatch is covered by adapter tests.
use super::*;
use nomifun_agent_contracts::{
    CapabilityCatalogMaterialization, CapabilityCatalogMaterializer, CapabilityContributions,
    CapabilityKind, CapabilityManifest, CapabilityOwner, CapabilityProvenance,
    CapabilityReleaseState, CapabilitySelection, CatalogAvailability, ExactRoleContractRef,
    ExactRoleProviderRef, InstallationRoleBinding, LocalizedMetadata, PackageRef,
    PlatformConstraint, RoleContractKey, RoleContractManifest, RoleMemberContract,
    RoleMemberRequirement, RoleProviderContribution, RoleProviderMemberContribution,
    RoleProviderSelection, RuntimeProfileKind, StrictJsonValue, capability_surface_declarations,
};
use nomifun_agent_kernel::{
    MaterializedCapability, MaterializedRoleContract, MaterializedRoleProvider,
};
use std::collections::BTreeMap;

use crate::OfficialTemplateCatalog;

const ROLE: &str = "system.test_context";
const MEMBER: &str = "platform.test_context";

struct Fixture {
    registry: MaterializedRegistry,
    catalog: CatalogSnapshot,
    environment: CompilerEnvironment,
    draft: AgentPresetDraftDto,
}

impl Fixture {
    fn new() -> Self {
        let manifest = CapabilityManifest {
            id: MEMBER.into(),
            contribution_id: format!("capability:{MEMBER}").into(),
            kind: CapabilityKind::ContextContributor,
            package: PackageRef {
                id: "platform.test".into(),
                version: "1.0.0".into(),
            },
            display: LocalizedMetadata {
                name: "Context".into(),
                description: "Fixture".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions {
                context_schema_refs: vec!["schema://platform.test/context@1".into()],
                ..Default::default()
            },
        };
        let reference = CapabilityRef {
            id: manifest.id.clone(),
        };
        let digest = digest_payload(&manifest).unwrap();
        let artifact_digest = digest_payload(&"fixture artifact").unwrap();
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::Bundled,
            source_identity: "platform.test".into(),
            source_digest: Some(artifact_digest.clone()),
        };
        let contribution_lock = ContributionLock {
            source_kind: ContributionSourceKind::PlatformBuiltin,
            source_identity: source.source_identity.clone().into(),
            mount_id: None,
            plugin_product_id: None,
            mcp_binding_id: None,
            contribution_id: manifest.contribution_id.clone(),
            contract_digest: digest.clone(),
        };
        let entry = CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
            manifest: manifest.clone(),
            provenance: CapabilityProvenance {
                owner: CapabilityOwner::Package {
                    package: manifest.package.clone(),
                },
                source_kind: contribution_lock.source_kind,
                source_identity: contribution_lock.source_identity.clone(),
                mount_id: None,
                plugin_product_id: None,
                mcp_binding_id: None,
                artifact_digest: Some(artifact_digest.clone()),
            },
            release_state: CapabilityReleaseState::PublishedActive,
            availability: BTreeMap::from([(
                CapabilityConsumer::Agent,
                CatalogAvailability::Active,
            )]),
        })
        .unwrap();
        let capability = MaterializedCapability {
            contribution_id: manifest.contribution_id.clone(),
            manifest,
            schema_digest: digest.clone(),
            contribution_lock,
            target_artifact_digest: artifact_digest,
            mount_id: "contract".into(),
            source,
        };
        let contract = RoleContractManifest {
            key: RoleContractKey {
                role_id: ROLE.into(),
                contract_version: "1.0.0".into(),
            },
            members: vec![RoleMemberContract {
                capability: reference.clone(),
                capability_manifest_digest: digest,
                requirement: RoleMemberRequirement::Required,
            }],
            serialized_target_resource_kind: None,
        };
        let exact_role = ExactRoleContractRef {
            key: contract.key.clone(),
            contract_digest: digest_payload(&contract).unwrap(),
        };
        let mut registry = MaterializedRegistry::empty();
        registry
            .capabilities
            .insert(MEMBER.into(), capability.clone());
        registry.capability_roles.insert(MEMBER.into(), ROLE.into());
        registry.role_contracts.insert(
            ROLE.into(),
            MaterializedRoleContract {
                manifest: contract,
                contract_digest: exact_role.contract_digest.clone(),
                mount_id: "contract".into(),
            },
        );
        for mount in ["builtin", "user"] {
            let contribution = RoleProviderContribution {
                role: exact_role.clone(),
                display: capability.manifest.display.clone(),
                members: BTreeMap::from([(
                    MEMBER.into(),
                    RoleProviderMemberContribution {
                        implementation: None,
                        supported_platforms: vec![PlatformConstraint::Any],
                        required_resource_kinds: BTreeSet::new(),
                    },
                )]),
            };
            registry.role_providers.insert(
                (ROLE.into(), mount.into()),
                MaterializedRoleProvider {
                    provider: ExactRoleProviderRef {
                        role: exact_role.clone(),
                        package: PackageRef {
                            id: format!("provider.{mount}").into(),
                            version: "1.0.0".into(),
                        },
                        mount_id: mount.into(),
                        contribution_digest: digest_payload(&contribution).unwrap(),
                    },
                    contribution,
                    source: PluginSourceMetadata {
                        source_kind: if mount == "builtin" {
                            PluginSourceKind::Bundled
                        } else {
                            PluginSourceKind::ManagedLocal
                        },
                        source_identity: mount.into(),
                        source_digest: Some(digest_payload(&mount).unwrap()),
                    },
                },
            );
        }
        let environment = CompilerEnvironment {
            resolver_version: "1.0.0".into(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: digest_payload(&"features").unwrap(),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::from([(
                ROLE.into(),
                InstallationRoleBinding {
                    selection: RoleProviderSelection {
                        role: exact_role,
                        provider_mount_id: "user".into(),
                    },
                    binding_version: 1,
                    updated_at_ms: 1,
                },
            )]),
            canonical_schema_manifest_digest: digest_payload(&"schema").unwrap(),
            target_contribution_manifest_digest: digest_payload(&"contributions").unwrap(),
            host_target: "test".into(),
            host_surface: "desktop".into(),
            availability_evidence_revision: "fixture".into(),
        };
    let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: reference.clone(),
                action_allowlist: BTreeSet::new(),
            }],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };
        Self {
            catalog: CatalogSnapshot {
                capabilities: vec![capability],
                formal_capability_entries: BTreeMap::from([(reference, entry)]),
                role_contracts: registry.role_contracts.values().cloned().collect(),
                role_providers: registry.role_providers.values().cloned().collect(),
                ..Default::default()
            },
            registry,
            environment,
            draft: AgentPresetDraftDto {
                preset_id: "preset-role-reuse".into(),
                display_name: "Role reuse".into(),
                description: None,
                source_template_key: None,
                current_revision: None,
                document: wire_cast(&payload).unwrap(),
            },
        }
    }

    fn compiler(&self) -> PresetRevisionCompiler {
        PresetRevisionCompiler::new()
            .with_materialized_registry(Arc::new(self.registry.clone()), self.environment.clone())
    }

    fn compile(
        &self,
        saved: Option<&(AgentPresetRevision, ResolvedSnapshotEnvelope)>,
    ) -> PresetCompilation {
        self.compiler()
            .compile(
                &"owner".into(),
                &self.draft,
                saved.map(|s| &s.0),
                saved.map(|s| &s.1),
                &self.catalog,
            )
            .unwrap()
    }

    fn save(&self) -> (AgentPresetRevision, ResolvedSnapshotEnvelope) {
        let compiled = self.compile(None);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let snapshot = compiled.snapshot.unwrap();
        (
            AgentPresetRevision {
                reference: compiled.candidate_revision_ref,
                payload: compiled.payload,
                contribution_locks: compiled.contribution_locks,
                created_by: "owner".into(),
                created_at_ms: snapshot.created_at_ms,
                reason: None,
            },
            snapshot,
        )
    }

    fn user_provider(&mut self) -> &mut MaterializedRoleProvider {
        self.registry
            .role_providers
            .get_mut(&(ROLE.into(), "user".into()))
            .unwrap()
    }

    fn add_context(&mut self, id: &str, selected: bool) {
        self.add_contribution(id, selected, false);
    }

    fn add_contribution(&mut self, id: &str, selected: bool, middleware: bool) {
        let mut capability = self.registry.capabilities[&MEMBER.into()].clone();
        capability.manifest.id = id.into();
        capability.manifest.contribution_id = format!("capability:{id}").into();
        if middleware {
            capability.manifest.contributions.actions = vec![nomifun_agent_contracts::model_middleware::action()];
        }
        capability.contribution_id = capability.manifest.contribution_id.clone();
        capability.schema_digest = digest_payload(&capability.manifest).unwrap();
        capability.contribution_lock.contribution_id = capability.contribution_id.clone();
        capability.contribution_lock.contract_digest = capability.schema_digest.clone();
        self.registry.capabilities.insert(id.into(), capability.clone());
        if selected {
            let reference = CapabilityRef { id: id.into() };
            self.draft.document.enabled_capabilities.push(wire_cast(&CapabilitySelection {
                capability: reference.clone(),
                action_allowlist: middleware.then(|| {
                    BTreeSet::from([nomifun_agent_contracts::model_middleware::ACTION_ID.into()])
                }).unwrap_or_default(),
            }).unwrap());
            let entry = CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
                manifest: capability.manifest.clone(),
                provenance: CapabilityProvenance {
                    owner: CapabilityOwner::Package { package: capability.manifest.package.clone() },
                    source_kind: capability.contribution_lock.source_kind,
                    source_identity: capability.contribution_lock.source_identity.clone(),
                    mount_id: None, plugin_product_id: None, mcp_binding_id: None,
                    artifact_digest: Some(capability.target_artifact_digest.clone()),
                },
                release_state: CapabilityReleaseState::PublishedActive,
                availability: BTreeMap::from([(CapabilityConsumer::Agent, CatalogAvailability::Active)]),
            }).unwrap();
            self.catalog.formal_capability_entries.insert(reference, entry);
            self.catalog.capabilities.push(capability);
        }
    }

    fn map_user_context(&mut self, id: &str) {
        self.add_context(id, false);
        let provider = self.user_provider();
        provider.contribution.members.get_mut(&MEMBER.into()).unwrap().implementation = Some(CapabilityRef {
            id: id.into(),
        });
        provider.provider.contribution_digest = digest_payload(&provider.contribution).unwrap();
        self.catalog.role_providers = self.registry.role_providers.values().cloned().collect();
    }
}

#[test]
fn consumer_validation_runs_on_new_and_unchanged_plans_without_rewriting_them() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let saved = fixture.save();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let digest = fixture.registry.registry_digest.clone();
    let compiler = fixture.compiler().with_consumer_validator(move |registry, snapshot| {
        assert_eq!(registry.registry_digest, digest);
        assert!(!snapshot.content.enabled_capabilities.is_empty());
        seen.fetch_add(1, Ordering::SeqCst);
        Err(ControlPlaneError::canonical(
            "CAPABILITY_UNAVAILABLE", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "fixture consumer rejects the selection",
        ))
    });
    for prior in [None, Some(&saved)] {
        let result = compiler.compile(
            &"owner".into(), &fixture.draft, prior.map(|p| &p.0), prior.map(|p| &p.1),
            &fixture.catalog,
        ).unwrap();
        assert!(result.snapshot.is_none());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "CAPABILITY_UNAVAILABLE");
        if prior.is_some() {
            assert_eq!(result.candidate_revision_ref, saved.0.reference);
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let accepted = fixture.compiler().with_consumer_validator(|_, _| Ok(()))
        .compile(&"owner".into(), &fixture.draft, Some(&saved.0), Some(&saved.1), &fixture.catalog)
        .unwrap();
    assert_eq!(accepted.snapshot.as_ref(), Some(&saved.1));
}

#[test]
fn selected_implementation_conflicts_are_checked_against_other_public_capabilities() {
    let mut fixture = Fixture::new();
    fixture.add_context("fixture.peer", true);
    fixture.map_user_context("fixture.implementation");
    fixture.registry.capabilities.get_mut(&"fixture.implementation".into()).unwrap()
        .manifest.conflicts.push(nomifun_agent_contracts::CapabilityConflict {
            capability: CapabilityRef { id: "fixture.peer".into() },
            reason: "requires exclusive context ownership".into(),
        });
    let rejected = fixture.compile(None);
    assert!(rejected.snapshot.is_none());
    assert!(rejected.diagnostics.iter().any(|d| d.message.contains("fixture.implementation") && d.message.contains("fixture.peer")),
        "{:?}", rejected.diagnostics);
    fixture.environment.installation_role_bindings.get_mut(&ROLE.into()).unwrap()
        .selection.provider_mount_id = "builtin".into();
    assert!(fixture.compile(None).diagnostics.is_empty(), "unselected Provider conflicts must not constrain builtin");
}

#[tokio::test]
async fn clean_save_rejects_legacy_conflicting_plan_without_rewriting_saved_revision() {
    use crate::{AgentControlPlane, ControlPlaneStore, InMemoryControlPlaneStore, StaticCatalogProvider};
    use nomifun_api_types::{CreateAgentPresetRequest, SaveAgentPresetRevisionRequest};

    let mut fixture = Fixture::new();
    fixture.add_context("fixture.peer", true);
    fixture.map_user_context("fixture.implementation");
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let control = AgentControlPlane::new(
        store.clone(), Arc::new(StaticCatalogProvider::new(fixture.catalog.clone())),
        OfficialTemplateCatalog::load().unwrap(), fixture.compiler(),
    );
    let owner = UserId::from("owner");
    let editor = control.create_preset(&owner, CreateAgentPresetRequest {
        display_name: "Conflicts".into(), description: None, fork_from_revision: None, document: None,
    }).await.unwrap();
    fixture.draft.preset_id = editor.draft.preset_id;
    let saved = fixture.save();
    store.append_revision(None, saved.0.clone(), saved.1.clone(), "Conflicts".into(), None).await.unwrap();
    // Model a legacy plan that the old facade-only conflict validator accepted.
    // Keep exact locks unchanged deliberately: authoring reuse must run the
    // canonical conflict check, not rely solely on a Provider lock difference.
    fixture.registry.capabilities.get_mut(&"fixture.implementation".into()).unwrap()
        .manifest.conflicts.push(nomifun_agent_contracts::CapabilityConflict {
            capability: CapabilityRef { id: "fixture.peer".into() },
            reason: "legacy plan skipped implementation conflicts".into(),
        });
    assert!(!KernelAgentPresetCompiler::role_providers_unchanged(
        &fixture.registry, &fixture.environment, &saved.0, &saved.1,
    ));
    let control = AgentControlPlane::new(
        store.clone(), Arc::new(StaticCatalogProvider::new(fixture.catalog.clone())),
        OfficialTemplateCatalog::load().unwrap(), fixture.compiler(),
    );
    fixture.draft.current_revision = Some(wire_cast(&saved.0.reference).unwrap());
    let result = control.save_revision(&owner, &fixture.draft.preset_id, SaveAgentPresetRevisionRequest {
        expected_current_revision: fixture.draft.current_revision.clone(),
        draft: fixture.draft.clone(), reason: None,
    }).await;
    let error = result.err().expect("a clean draft must not reuse an incompatible legacy plan");
    let details = error.details().unwrap();
    assert!(details["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        let message = diagnostic["message"].as_str().unwrap_or_default();
        message.contains("conflict") && message.contains("fixture.implementation") && message.contains("fixture.peer")
    }), "{details}");
    assert_eq!(store.get_snapshot(&saved.0.reference).await.unwrap().unwrap(), saved.1);
    assert_eq!(store.get_revision(&saved.0.reference).await.unwrap().unwrap(), saved.0);
}

#[test]
fn conflicts_between_two_selected_roles_are_checked_after_both_providers_are_resolved() {
    let mut fixture = Fixture::new();
    fixture.map_user_context("fixture.first-implementation");
    fixture.add_context("fixture.peer", true);
    fixture.add_context("fixture.second-implementation", false);
    let peer_id = nomifun_agent_contracts::CapabilityId::from("fixture.peer");
    let role_id = nomifun_agent_contracts::ExecutionRoleId::from("system.peer");
    let mut contract = fixture.registry.role_contracts[&ROLE.into()].clone();
    contract.manifest.key.role_id = role_id.clone();
    contract.manifest.members[0].capability.id = peer_id.clone();
    contract.manifest.members[0].capability_manifest_digest = fixture.registry.capabilities[&peer_id].schema_digest.clone();
    contract.contract_digest = digest_payload(&contract.manifest).unwrap();
    let exact = ExactRoleContractRef { key: contract.manifest.key.clone(), contract_digest: contract.contract_digest.clone() };
    let mut provider = fixture.user_provider().clone();
    let mut member = provider.contribution.members.remove(&MEMBER.into()).unwrap();
    member.implementation.as_mut().unwrap().id = "fixture.second-implementation".into();
    provider.contribution.members.insert(peer_id.clone(), member);
    provider.contribution.role = exact.clone();
    provider.provider.role = exact.clone();
    provider.provider.contribution_digest = digest_payload(&provider.contribution).unwrap();
    fixture.registry.capability_roles.insert(peer_id, role_id.clone());
    fixture.registry.role_contracts.insert(role_id.clone(), contract);
    fixture.registry.role_providers.insert((role_id.clone(), "user".into()), provider);
    fixture.environment.installation_role_bindings.insert(role_id, InstallationRoleBinding {
        selection: RoleProviderSelection { role: exact, provider_mount_id: "user".into() },
        binding_version: 1, updated_at_ms: 1,
    });
    let allowed = fixture.compile(None);
    assert!(allowed.diagnostics.is_empty(), "{:?}", allowed.diagnostics);
    assert_eq!(allowed.snapshot.unwrap().content.resolved_role_providers.len(), 2);
    fixture.registry.capabilities.get_mut(&"fixture.first-implementation".into()).unwrap()
        .manifest.conflicts.push(nomifun_agent_contracts::CapabilityConflict {
            capability: CapabilityRef { id: "fixture.second-implementation".into() },
            reason: "these two selected implementations cannot coexist".into(),
        });
    let rejected = fixture.compile(None);
    assert!(rejected.snapshot.is_none());
    assert!(rejected.diagnostics.iter().any(|d| d.message.contains("fixture.first-implementation") && d.message.contains("fixture.second-implementation")),
        "{:?}", rejected.diagnostics);
}

#[test]
fn middleware_order_changes_frozen_profile_reuses_clean_save_and_rejects_other_contributions() {
    let mut fixture = Fixture::new();
    for id in ["request.a", "request.z"] {
        fixture.add_contribution(id, true, true);
    }
    let old = fixture.save();
    fixture.draft.document.middleware_order = vec!["request.z".into()];
    let changed = fixture.compile(Some(&old));
    assert!(changed.diagnostics.is_empty(), "{:?}", changed.diagnostics);
    let snapshot = changed.snapshot.unwrap();
    assert_eq!(snapshot.content.middleware_order, vec![nomifun_agent_contracts::CapabilityId::from("request.z")]);
    assert_ne!(snapshot.snapshot_ref, old.1.snapshot_ref);
    assert_ne!(snapshot.content.compiled_runtime_profile_digest, old.1.content.compiled_runtime_profile_digest);
    assert_eq!(snapshot.content.capability_allowlist, old.1.content.capability_allowlist);
    assert_eq!(snapshot.content.resolved_role_providers, old.1.content.resolved_role_providers);
    assert!(old.1.content.middleware_order.is_empty());
    let saved = fixture.save();
    assert_eq!(fixture.compile(Some(&saved)).snapshot.unwrap(), saved.1);
    let round_trip: nomifun_api_types::AgentPresetDocumentDto = wire_cast(&saved.0.payload).unwrap();
    assert_eq!(round_trip.middleware_order, fixture.draft.document.middleware_order);
    for order in [vec![MEMBER], vec!["request.z", "request.z"], vec!["missing"]] {
        fixture.draft.document.middleware_order = order.into_iter().map(Into::into).collect();
        let rejected = fixture.compile(Some(&saved));
        assert!(rejected.snapshot.is_none());
        assert!(rejected.diagnostics.iter().any(|d| d.message.contains("middleware_order")), "{:?}", rejected.diagnostics);
    }
}

#[test]
fn context_order_round_trips_and_changes_snapshot_without_reselecting_provider() {
    let mut fixture = Fixture::new();
    let old = fixture.save();
    fixture.draft.document.context_order = vec![MEMBER.into()];
    let changed = fixture.compile(Some(&old));
    assert!(changed.diagnostics.is_empty(), "{:?}", changed.diagnostics);
    let snapshot = changed.snapshot.unwrap();
    assert_eq!(snapshot.content.context_order, vec![nomifun_agent_contracts::CapabilityId::from(MEMBER)]);
    assert_ne!(snapshot.snapshot_ref, old.1.snapshot_ref);
    assert_ne!(snapshot.content.compiled_runtime_profile_digest, old.1.content.compiled_runtime_profile_digest);
    assert_eq!(snapshot.content.resolved_role_providers, old.1.content.resolved_role_providers);
    assert_eq!(snapshot.content.capability_allowlist, old.1.content.capability_allowlist);
    assert!(old.1.content.context_order.is_empty());
    let saved = fixture.save();
    assert_eq!(fixture.compile(Some(&saved)).snapshot.unwrap(), saved.1);
    let round_trip: nomifun_api_types::AgentPresetDocumentDto = wire_cast(&saved.0.payload).unwrap();
    assert_eq!(round_trip.context_order, fixture.draft.document.context_order);
    fixture.draft.document.context_order.push(MEMBER.into());
    assert!(!fixture.compile(Some(&saved)).diagnostics.is_empty());
}

#[test]
fn clean_save_reuses_exact_snapshot_despite_unrelated_provider_or_binding_metadata_changes() {
    let mut fixture = Fixture::new();
    let saved = fixture.save();
    fixture.registry.generation += 1;
    fixture
        .registry
        .role_providers
        .remove(&(ROLE.into(), "builtin".into()));
    let binding = fixture
        .environment
        .installation_role_bindings
        .get_mut(&ROLE.into())
        .unwrap();
    binding.binding_version += 1;
    binding.updated_at_ms += 1;
    let compiled = fixture.compile(Some(&saved));
    assert!(compiled.diagnostics.is_empty());
    assert_eq!(compiled.candidate_revision_ref, saved.0.reference);
    assert_eq!(compiled.snapshot.unwrap(), saved.1);
}

#[test]
fn skill_artifact_drift_recompiles_clean_save_even_when_body_and_contract_are_unchanged() {
    use nomifun_agent_contracts::{LogicalArtifactRef, SkillDefinition};
    use nomifun_agent_kernel::MaterializedSkill;
    let mut fixture = Fixture::new();
    let definition = SkillDefinition {
        id: "plugin.guide".into(), version: "1.0.0".into(),
        package: PackageRef { id: "plugin.guide".into(), version: "1.0.0".into() },
        display: LocalizedMetadata { name: "Guide".into(), description: "Guide".into(), localized_names: BTreeMap::new(), localized_descriptions: BTreeMap::new() },
        body_ref: LogicalArtifactRef { artifact_id: "guide".into(), normalized_relative_path: "resources/guide.md".into(), digest: digest_payload(&"body").unwrap() },
        resources: vec![], requires_capabilities: vec![],
        supported_surfaces: capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent]),
    };
    let contract_digest = digest_payload(&definition).unwrap();
    let target = digest_payload(&"artifact-a").unwrap();
    let skill = MaterializedSkill {
        definition, contribution_id: "skill:plugin.guide".into(), contract_digest: contract_digest.clone(),
        contribution_lock: ContributionLock {
            source_kind: ContributionSourceKind::PluginMount, source_identity: "guide-mount".into(),
            mount_id: Some("guide-mount".into()), plugin_product_id: None, mcp_binding_id: None,
            contribution_id: "skill:plugin.guide".into(), contract_digest,
        },
        target_artifact_digest: target.clone(), mount_id: "guide-mount".into(),
        source: PluginSourceMetadata { source_kind: PluginSourceKind::ManagedLocal, source_identity: "guide-mount".into(), source_digest: Some(target) },
    };
    fixture.registry.skills.insert(skill.definition.id.clone(), skill.clone());
    fixture.catalog.skills.push(skill);
    fixture.draft.document.skill_bindings.push(nomifun_api_types::ExactCatalogRefDto { id: "plugin.guide".into(), version: "1.0.0".into() });
    let saved = fixture.save();
    assert_eq!(fixture.compile(Some(&saved)).snapshot.unwrap(), saved.1);
    let changed = fixture.registry.skills.get_mut(&"plugin.guide".into()).unwrap();
    changed.target_artifact_digest = digest_payload(&"artifact-b").unwrap();
    changed.source.source_digest = Some(changed.target_artifact_digest.clone());
    fixture.catalog.skills = vec![changed.clone()];
    let result = fixture.compile(Some(&saved));
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.candidate_revision_ref.revision, saved.0.reference.revision + 1);
    let snapshot = result.snapshot.unwrap();
    assert_eq!(snapshot.content.skill_locks[0].body_digest, saved.1.content.skill_locks[0].body_digest);
    assert_ne!(snapshot.content.skill_locks[0].target_artifact_digest, saved.1.content.skill_locks[0].target_artifact_digest);
    fixture.registry.skills.clear();
    fixture.catalog.skills.clear();
    let withdrawn = fixture.compile(Some(&saved));
    assert!(withdrawn.snapshot.is_none());
    assert!(!withdrawn.diagnostics.is_empty());
}

#[test]
fn unchanged_draft_recompiles_selected_provider_artifact_and_contribution_changes() {
    for change in ["artifact", "contribution", "package", "source"] {
        let mut fixture = Fixture::new();
        let saved = fixture.save();
        let provider = fixture.user_provider();
        match change {
            "artifact" => {
                provider.source.source_digest = Some(digest_payload(&"new artifact").unwrap())
            }
            "contribution" => {
                provider.contribution.display.description = "Updated implementation".into();
                provider.provider.contribution_digest =
                    digest_payload(&provider.contribution).unwrap();
            }
            "package" => provider.provider.package.version = "1.0.1".into(),
            "source" => provider.source.source_identity = "new-source".into(),
            _ => unreachable!(),
        }
        let expected = provider.clone();
        let compiled = fixture.compile(Some(&saved));
        assert!(
            compiled.diagnostics.is_empty(),
            "{change}: {:?}",
            compiled.diagnostics
        );
        assert_eq!(
            compiled.candidate_revision_ref.revision,
            saved.0.reference.revision + 1,
            "{change}"
        );
        let snapshot = compiled.snapshot.unwrap();
        assert_ne!(snapshot.snapshot_ref, saved.1.snapshot_ref, "{change}");
        let lock = &snapshot.content.resolved_role_providers[&ROLE.into()];
        assert_eq!(lock.provider, expected.provider);
        assert_eq!(lock.source, expected.source);
        snapshot.validate().unwrap();
        saved.1.validate().unwrap();
    }
}

#[tokio::test]
async fn save_refreshes_stale_resource_projection_without_changing_provider_or_old_revision() {
    use crate::{
        AgentControlPlane, ControlPlaneStore, InMemoryControlPlaneStore, StaticCatalogProvider,
    };
    use nomifun_api_types::{CreateAgentPresetRequest, SaveAgentPresetRevisionRequest};

    let mut fixture = Fixture::new();
    let provider = fixture.user_provider();
    provider
        .contribution
        .members
        .get_mut(&MEMBER.into())
        .unwrap()
        .required_resource_kinds
        .insert("fixture.index".into());
    provider.provider.contribution_digest = digest_payload(&provider.contribution).unwrap();
    fixture.catalog.role_providers = fixture.registry.role_providers.values().cloned().collect();
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let control = AgentControlPlane::new(
        store.clone(),
        Arc::new(StaticCatalogProvider::new(fixture.catalog.clone())),
        OfficialTemplateCatalog::load().unwrap(),
        fixture.compiler(),
    );
    let owner = UserId::from("owner");
    let editor = control
        .create_preset(
            &owner,
            CreateAgentPresetRequest {
                display_name: "Resource projection".into(),
                description: None,
                fork_from_revision: None,
                document: None,
            },
        )
        .await
        .unwrap();
    fixture.draft.preset_id = editor.draft.preset_id;
    let (revision, mut legacy) = fixture.save();
    let expected_requirements = legacy.content.enabled_capabilities[0]
        .required_resource_kinds
        .clone();
    assert_eq!(
        expected_requirements,
        BTreeSet::from(["fixture.index".into()])
    );
    // Seed a syntactically valid but stale earlier requirement projection.
    // This is not an artifact upgrade: all exact Provider locks stay identical.
    legacy.content.enabled_capabilities[0]
        .required_resource_kinds
        .clear();
    legacy.snapshot_ref.snapshot_digest = digest_payload(&legacy.content).unwrap();
    legacy.validate().unwrap();
    store
        .append_revision(
            None,
            revision.clone(),
            legacy.clone(),
            "Resource projection".into(),
            None,
        )
        .await
        .unwrap();
    let mut draft = fixture.draft;
    draft.current_revision = Some(wire_cast(&revision.reference).unwrap());
    let preset_id = draft.preset_id.clone();
    let updated = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(),
                draft: draft.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    let updated_ref: PresetRevisionRef = wire_cast(&updated.revision.reference).unwrap();
    assert_eq!(updated_ref.revision, revision.reference.revision + 1);
    let snapshot = store.get_snapshot(&updated_ref).await.unwrap().unwrap();
    assert_eq!(
        snapshot.content.resolved_role_providers,
        legacy.content.resolved_role_providers
    );
    assert_eq!(
        snapshot.content.enabled_capabilities[0].required_resource_kinds,
        expected_requirements
    );
    assert_eq!(
        snapshot.content.required_resource_kinds,
        expected_requirements
    );
    assert_eq!(
        store
            .get_snapshot(&revision.reference)
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    assert_eq!(
        store
            .get_revision(&revision.reference)
            .await
            .unwrap()
            .unwrap(),
        revision
    );
    draft.current_revision = Some(updated.revision.reference.clone());
    let clean = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(),
                draft,
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(clean.revision.reference, updated.revision.reference);
    assert_eq!(clean.resolved_snapshot_ref, updated.resolved_snapshot_ref);
}

#[test]
fn unchanged_draft_rejects_withdrawn_incompatible_or_incomplete_selected_provider() {
    for change in ["withdrawn", "contract", "member", "platform"] {
        let mut fixture = Fixture::new();
        let saved = fixture.save();
        match change {
            "withdrawn" => {
                fixture
                    .registry
                    .role_providers
                    .remove(&(ROLE.into(), "user".into()));
            }
            "contract" => {
                fixture
                    .registry
                    .role_contracts
                    .get_mut(&ROLE.into())
                    .unwrap()
                    .contract_digest = digest_payload(&"new contract").unwrap();
            }
            "member" => {
                fixture.user_provider().contribution.members.clear();
            }
            "platform" => {
                fixture
                    .user_provider()
                    .contribution
                    .members
                    .get_mut(&MEMBER.into())
                    .unwrap()
                    .supported_platforms = vec![PlatformConstraint::Targets {
                    host_targets: BTreeSet::from(["other".into()]),
                    host_surfaces: BTreeSet::new(),
                }];
            }
            _ => unreachable!(),
        }
        let compiled = fixture.compile(Some(&saved));
        assert!(
            compiled.snapshot.is_none(),
            "{change} must not reuse the saved Snapshot"
        );
        let expected_code = match change {
            "platform" => "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
            _ => "CAPABILITY_NOT_MATERIALIZED",
        };
        assert!(
            compiled.diagnostics.iter().any(|d| d.code == expected_code),
            "{change}: {:?}",
            compiled.diagnostics
        );
        assert_eq!(
            saved.1.content.resolved_role_providers[&ROLE.into()]
                .provider
                .mount_id
                .as_ref(),
            "user"
        );
    }
}

#[test]
fn new_save_uses_changed_default_but_explicit_override_and_old_snapshot_do_not_drift() {
    for overridden in [false, true] {
        let mut fixture = Fixture::new();
        if overridden {
            fixture
                .draft
                .document
                .system_role_provider_overrides
                .insert(
                    ROLE.into(),
                    wire_cast(
                        &fixture.environment.installation_role_bindings[&ROLE.into()].selection,
                    )
                    .unwrap(),
                );
        }
        let saved = fixture.save();
        fixture
            .environment
            .installation_role_bindings
            .get_mut(&ROLE.into())
            .unwrap()
            .selection
            .provider_mount_id = "builtin".into();
        let compiled = fixture.compile(Some(&saved));
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let snapshot = compiled.snapshot.unwrap();
        let expected_mount = if overridden { "user" } else { "builtin" };
        assert_eq!(
            snapshot.content.resolved_role_providers[&ROLE.into()]
                .provider
                .mount_id
                .as_ref(),
            expected_mount
        );
        assert_eq!(
            compiled.candidate_revision_ref.revision,
            saved.0.reference.revision + u64::from(!overridden)
        );
        assert_eq!(
            saved.1.content.resolved_role_providers[&ROLE.into()]
                .provider
                .mount_id
                .as_ref(),
            "user"
        );
        // Removing the current default never clears an explicit user override.
        fixture.environment.installation_role_bindings.clear();
        let compiled = fixture.compile(Some(&saved));
        assert_eq!(compiled.snapshot.is_some(), overridden);
        if !overridden {
            assert!(compiled.diagnostics.iter().any(
                |d| d.code == "CAPABILITY_NOT_MATERIALIZED" && d.message.contains("not bound")
            ));
        }
    }
}

#[test]
fn new_role_membership_invalidates_a_snapshot_with_no_provider_lock() {
    let mut fixture = Fixture::new();
    fixture.registry.capability_roles.clear();
    let saved = fixture.save();
    assert!(saved.1.content.resolved_role_providers.is_empty());
    fixture
        .registry
        .capability_roles
        .insert(MEMBER.into(), ROLE.into());
    let compiled = fixture.compile(Some(&saved));
    assert!(compiled.diagnostics.is_empty());
    assert_eq!(
        compiled.candidate_revision_ref.revision,
        saved.0.reference.revision + 1
    );
    assert!(
        compiled
            .snapshot
            .unwrap()
            .content
            .resolved_role_providers
            .contains_key(&ROLE.into())
    );
}

#[tokio::test]
async fn save_read_upgrade_and_withdrawal_preserve_old_revisions_and_never_fallback() {
    use crate::{
        AgentControlPlane, ControlPlaneStore, InMemoryControlPlaneStore, StaticCatalogProvider,
    };
    use nomifun_api_types::{CreateAgentPresetRequest, SaveAgentPresetRevisionRequest};
    use std::sync::RwLock;

    let fixture = Fixture::new();
    let registry = Arc::new(RwLock::new(fixture.registry.clone()));
    let registry_reader = registry.clone();
    let templates = OfficialTemplateCatalog::load().unwrap();
    let compiler = PresetRevisionCompiler::new().with_canonical_registry(
        Arc::new(move || Ok(Arc::new(registry_reader.read().unwrap().clone()))),
        fixture.environment.clone(),
    );
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let control = AgentControlPlane::new(
        store.clone(),
        Arc::new(StaticCatalogProvider::new(fixture.catalog)),
        templates,
        compiler,
    );
    let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
    let editor = control
        .create_preset(
            &owner,
            CreateAgentPresetRequest {
                display_name: "Provider reuse".into(),
                description: None,
                fork_from_revision: None,
                document: None,
            },
        )
        .await
        .unwrap();
    let mut draft = editor.draft;
    draft.document = fixture.draft.document;
    let preset_id = draft.preset_id.clone();
    let first = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    let first_ref: PresetRevisionRef = wire_cast(&first.revision.reference).unwrap();
    let first_snapshot = store.get_snapshot(&first_ref).await.unwrap().unwrap();
    draft.current_revision = Some(first.revision.reference.clone());
    let clean = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: draft.current_revision.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(clean.revision.reference, first.revision.reference);
    assert_eq!(clean.resolved_snapshot_ref, first.resolved_snapshot_ref);

    let new_digest = digest_payload(&"upgraded installed artifact").unwrap();
    registry
        .write()
        .unwrap()
        .role_providers
        .get_mut(&(ROLE.into(), "user".into()))
        .unwrap()
        .source
        .source_digest = Some(new_digest.clone());
    let updated = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: draft.current_revision.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated.revision.reference.revision,
        first.revision.reference.revision + 1
    );
    let updated_ref: PresetRevisionRef = wire_cast(&updated.revision.reference).unwrap();
    let updated_snapshot = store.get_snapshot(&updated_ref).await.unwrap().unwrap();
    assert_eq!(
        updated_snapshot.content.resolved_role_providers[&ROLE.into()]
            .source
            .source_digest,
        Some(new_digest)
    );
    assert_eq!(
        store.get_snapshot(&first_ref).await.unwrap().unwrap(),
        first_snapshot
    );
    assert_eq!(
        control
            .get_revision(&owner, &preset_id, first_ref.revision)
            .await
            .unwrap(),
        first.revision
    );

    registry
        .write()
        .unwrap()
        .role_providers
        .remove(&(ROLE.into(), "user".into()));
    draft.current_revision = Some(updated.revision.reference.clone());
    let error = control
        .save_revision(
            &owner,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: draft.current_revision.clone(),
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().as_ref(), "PRESET_REVISION_SAVE_FAILED");
    let details = error.details().unwrap();
    assert_eq!(
        details["diagnostics"][0]["code"],
        "CAPABILITY_NOT_MATERIALIZED"
    );
    assert!(
        details["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("user")
    );
    assert_eq!(
        store
            .get_preset(&preset_id.into())
            .await
            .unwrap()
            .unwrap()
            .preset
            .current_stable_revision,
        Some(updated_ref.clone())
    );
    assert!(
        store
            .get_revision_number(&updated_ref.preset_id, updated_ref.revision + 1)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.get_snapshot(&updated_ref).await.unwrap().unwrap(),
        updated_snapshot
    );
    assert!(
        registry
            .read()
            .unwrap()
            .role_provider(&ROLE.into(), &"builtin".into())
            .is_some(),
        "builtin was available but must not be selected as fallback"
    );
}
