use super::*;
use crate::{
    ActiveCapabilitySetSnapshot, CapabilityDependencyCall, CapabilityDependencyCaller,
    CompiledSnapshot, MaterializedRegistry,
};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};

const CHILD: &str = "sample.child";
const GRANDCHILD: &str = "sample.grandchild";
const PEER: &str = "sample.peer";

#[path = "context_dependency_tests.rs"]
mod context;

struct Probe {
    contexts: Mutex<Vec<CapabilityInvocationContext>>,
    entered: mpsc::UnboundedSender<CapabilityId>,
    resume: Notify,
    dropped: Arc<AtomicUsize>,
}

struct DropMark(Arc<AtomicUsize>);
impl Drop for DropMark {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl CapabilityHandler for Probe {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        let _drop = DropMark(Arc::clone(&self.dropped));
        self.contexts.lock().unwrap().push(context.clone());
        self.entered.send(context.capability_id.clone()).unwrap();
        if input.0["pause"] == true {
            self.resume.notified().await;
        }
        if let Some(next) = input.0["next"].as_str() {
            let call = call(next, input.0.get("input").cloned().unwrap_or(json!({})));
            let result = context.dependencies.invoke(call.clone()).await?;
            if input.0["twice"] == true {
                return context.dependencies.invoke(call).await;
            }
            Ok(result)
        } else {
            Ok(StrictJsonValue(
                json!({"reached": context.capability_id.as_ref()}),
            ))
        }
    }
}

fn call(target: &str, input: serde_json::Value) -> CapabilityDependencyCall {
    CapabilityDependencyCall {
        capability_id: CapabilityId::from(target),
        action_id: ActionId::from(SAMPLE_ACTION),
        call_key: "one-effect".into(),
        input: StrictJsonValue(input),
    }
}

struct Fixture {
    kernel: KernelRegistry,
    registrations: Vec<PluginRegistration>,
    snapshot: CompiledSnapshot,
    active: ActiveCapabilitySetSnapshot,
    probe: Arc<Probe>,
    events: mpsc::UnboundedReceiver<CapabilityId>,
}

impl Fixture {
    fn use_role_parent(&mut self, mapped: bool) {
        let mut facade = self.registrations[0]
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities[0]
            .clone();
        let private_dependencies = facade.requires.clone();
        if mapped {
            facade.requires.clear();
            self.registrations[0]
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities[0] = facade.clone();
        }
        let contract = RoleContractManifest {
            key: RoleContractKey {
                role_id: SAMPLE_ROLE.into(),
                contract_version: VERSION.into(),
            },
            members: vec![RoleMemberContract {
                capability: CapabilityRef {
                    id: facade.id.clone(),
                    version: facade.version.clone(),
                },
                capability_manifest_digest: digest_payload(&facade).unwrap(),
                requirement: RoleMemberRequirement::Required,
            }],
            serialized_target_resource_kind: None,
        };
        let role = ExactRoleContractRef {
            key: contract.key.clone(),
            contract_digest: digest_payload(&contract).unwrap(),
        };
        let mut root = PluginRegistration::new(self.registrations[0].metadata.clone());
        root.metadata.manifest.payload.contributions.role_contracts = vec![contract];
        refresh_manifest(&mut root);
        self.registrations[0] = root;

        let base = registration_for(
            "sample.provider",
            "sample-provider",
            "sample.provider.impl",
            "sample.provider.skill",
            "sample.provider.server",
            "",
        );
        let mut provider = PluginRegistration::new(base.metadata);
        let mut implementation = facade.clone();
        implementation.requires = private_dependencies;
        implementation.package = package_ref("sample.provider");
        implementation.id = "sample.provider.impl".into();
        implementation.contribution_id = "capability:sample.provider.impl".into();
        let implementation_ref = CapabilityRef {
            id: implementation.id.clone(),
            version: implementation.version.clone(),
        };
        provider.metadata.manifest.payload.contributions = PackageContributions {
            capabilities: if mapped {
                vec![implementation]
            } else {
                Vec::new()
            },
            role_providers: vec![RoleProviderContribution {
                role: role.clone(),
                display: display("Dependency provider", "Selected parent implementation"),
                members: BTreeMap::from([(
                    facade.id.clone(),
                    RoleProviderMemberContribution {
                        implementation: mapped.then_some(implementation_ref.clone()),
                        supported_platforms: vec![PlatformConstraint::Any],
                        required_resource_kinds: facade.contributions.resource_kinds.clone(),
                    },
                )]),
            }],
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
        };
        if mapped {
            provider
                .add_capability_handler(implementation_ref.id, self.probe.clone())
                .unwrap();
        }
        provider
            .add_role_action_handler(SAMPLE_ROLE.into(), facade.id, self.probe.clone())
            .unwrap();
        refresh_manifest(&mut provider);
        self.registrations.push(provider);
        let materialized = self.kernel.replace_all(self.registrations.clone()).unwrap();
        let mut revision = sample_revision("user-a");
        revision.payload.skill_bindings.clear();
        revision.payload.system_role_provider_overrides.insert(
            SAMPLE_ROLE.into(),
            nomifun_agent_contracts::RoleProviderSelection {
                role,
                provider_mount_id: "sample-provider".into(),
            },
        );
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let mut environment = compiler_environment(materialized.registry_digest.clone());
        environment.host_surface = "test".into();
        self.snapshot = AgentPresetCompiler::compile(
            &materialized,
            &environment,
            compile_request(revision, principal("user-a")),
        )
        .unwrap()
        .with_target_resource_bindings(
            &principal("user-a"),
            vec![resource_binding("user-a"), role_resource_binding("user-a")],
        )
        .unwrap();
        self.active = SessionCapabilityState::new(&self.snapshot)
            .snapshot()
            .unwrap();
    }

    fn new() -> Self {
        let kernel = KernelRegistry::new(
            MaterializationPolicy::stable_with_test_fixtures(VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap();
        let (entered, events) = mpsc::unbounded_channel();
        let probe = Arc::new(Probe {
            contexts: Mutex::new(Vec::new()),
            entered,
            resume: Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let registrations = [
            (SAMPLE_CAPABILITY, Some(CHILD)),
            (CHILD, Some(GRANDCHILD)),
            (GRANDCHILD, None),
            (PEER, None),
        ]
        .into_iter()
        .map(|(id, dependency)| {
            let base = registration_for(
                id,
                id,
                id,
                &format!("{id}.skill"),
                &format!("{id}.server"),
                "",
            );
            let mut registration = PluginRegistration::new(base.metadata);
            let contributions = &mut registration.metadata.manifest.payload.contributions;
            contributions.skills.clear();
            contributions.mcp_tools.clear();
            let capability = &mut contributions.capabilities[0];
            capability.requires = dependency
                .into_iter()
                .map(|id| CapabilityRef {
                    id: id.into(),
                    version: VERSION.into(),
                })
                .collect();
            if id == CHILD {
                capability.contributions.resource_kinds =
                    BTreeSet::from([SAMPLE_ROLE_RESOURCE_KIND.into()]);
            } else if id != SAMPLE_CAPABILITY {
                capability.contributions.resource_kinds.clear();
            }
            refresh_manifest(&mut registration);
            registration
                .add_capability_handler(id.into(), probe.clone())
                .unwrap();
            registration
        })
        .collect::<Vec<_>>();
        let materialized = kernel.replace_all(registrations.clone()).unwrap();
        let mut revision = sample_revision("user-a");
        revision.payload.skill_bindings.clear();
        revision
            .payload
            .enabled_capabilities
            .push(nomifun_agent_contracts::CapabilitySelection {
                capability: CapabilityRef {
                    id: PEER.into(),
                    version: VERSION.into(),
                },
                action_allowlist: BTreeSet::from([SAMPLE_ACTION.into()]),
            });
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &compiler_environment(materialized.registry_digest.clone()),
            compile_request(revision, principal("user-a")),
        )
        .unwrap()
        .with_target_resource_bindings(
            &principal("user-a"),
            vec![resource_binding("user-a"), role_resource_binding("user-a")],
        )
        .unwrap();
        let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
        Self {
            kernel,
            registrations,
            snapshot,
            active,
            probe,
            events,
        }
    }

    fn request(&self, input: serde_json::Value) -> CapabilityInvocationRequest {
        let mut request = invocation(
            &self.snapshot,
            principal("user-a"),
            self.active.generation,
            "",
        );
        request.input = StrictJsonValue(input);
        request
    }

    async fn invoke(&self, input: serde_json::Value) -> Result<StrictJsonValue, KernelError> {
        self.kernel
            .invoke(&self.snapshot, &self.active, self.request(input))
            .await
    }

    fn caller(&self, id: &str) -> CapabilityDependencyCaller {
        self.probe
            .contexts
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|context| context.capability_id.as_ref() == id)
            .unwrap()
            .dependencies
            .clone()
    }

    async fn entered(&mut self, id: &str) {
        let actual = tokio::time::timeout(Duration::from_secs(3), self.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual.as_ref(), id);
    }

    fn spawn_parent(&self) -> tokio::task::JoinHandle<Result<StrictJsonValue, KernelError>> {
        let (kernel, snapshot, active, request) = (
            self.kernel.clone(),
            self.snapshot.clone(),
            self.active.clone(),
            self.request(json!({"pause":true})),
        );
        tokio::spawn(async move { kernel.invoke(&snapshot, &active, request).await })
    }
}

fn code(error: KernelError, expected: &str) {
    assert_eq!(error.canonical_code().as_ref(), expected, "{error:?}");
}

fn current_revision(
    fixture: &Fixture,
    registry: &MaterializedRegistry,
) -> nomifun_agent_contracts::AgentPresetRevision {
    let mut revision = sample_revision("user-a");
    revision.payload.skill_bindings.clear();
    revision.payload.enabled_capabilities = fixture
        .snapshot
        .content()
        .contributions()
        .map(|capability| nomifun_agent_contracts::CapabilitySelection {
            capability: capability.capability.clone(),
            action_allowlist: fixture
                .snapshot
                .policy(&capability.capability.id)
                .unwrap()
                .allowed_actions
                .clone(),
        })
        .collect();
    revision.contribution_locks = revision
        .payload
        .enabled_capabilities
        .iter()
        .map(|value| {
            registry
                .capability(&value.capability.id)
                .unwrap()
                .contribution_lock
                .clone()
        })
        .collect();
    for (role, lock) in &fixture.snapshot.content().resolved_role_providers {
        revision.payload.system_role_provider_overrides.insert(
            role.clone(),
            nomifun_agent_contracts::RoleProviderSelection {
                role: lock.provider.role.clone(),
                provider_mount_id: lock.provider.mount_id.clone(),
            },
        );
    }
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

#[tokio::test]
async fn dependency_is_private_unless_explicitly_selected_and_stale_graph_cannot_be_reused() {
    let fixture = Fixture::new();
    let registry = fixture.kernel.snapshot().unwrap();
    let mut revision = current_revision(&fixture, &registry);
    let env = compiler_environment(registry.registry_digest.clone());
    assert!(AgentPresetCompiler::role_providers_unchanged(
        &registry,
        &env,
        &revision,
        &fixture.snapshot.envelope
    ));
    let mut request = fixture.request(json!({}));
    request.capability_id = CHILD.into();
    request.resource_binding_ids = fixture
        .snapshot
        .policy(&CHILD.into())
        .unwrap()
        .resource_binding_ids
        .clone();
    assert!(matches!(
        fixture
            .kernel
            .invoke(&fixture.snapshot, &fixture.active, request)
            .await,
        Err(KernelError::CapabilityNotInPreset { .. })
    ));
    assert!(fixture.probe.contexts.lock().unwrap().is_empty());
    for drift in 0..2 {
        let mut stale = fixture.snapshot.envelope.clone();
        let root = stale
            .content
            .enabled_capabilities
            .iter_mut()
            .find(|value| value.capability.id.as_ref() == SAMPLE_CAPABILITY)
            .unwrap();
        if drift == 0 {
            root.dependency_refs.clear();
        } else {
            root.consumption = nomifun_agent_contracts::CapabilityConsumption::Dependency;
        }
        assert!(!AgentPresetCompiler::role_providers_unchanged(
            &registry, &env, &revision, &stale
        ));
    }
    revision
        .payload
        .enabled_capabilities
        .push(nomifun_agent_contracts::CapabilitySelection {
            capability: CapabilityRef {
                id: CHILD.into(),
                version: VERSION.into(),
            },
            action_allowlist: BTreeSet::from([SAMPLE_ACTION.into()]),
        });
    revision.contribution_locks.push(
        registry
            .capability(&CHILD.into())
            .unwrap()
            .contribution_lock
            .clone(),
    );
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let compiled = AgentPresetCompiler::compile(
        &registry,
        &env,
        compile_request(revision, principal("user-a")),
    )
    .unwrap()
    .with_target_resource_bindings(
        &principal("user-a"),
        vec![resource_binding("user-a"), role_resource_binding("user-a")],
    )
    .unwrap();
    assert!(
        compiled
            .resolved_capability(&CHILD.into())
            .unwrap()
            .consumption
            .is_contribution()
    );
    assert!(
        !compiled
            .resolved_capability(&GRANDCHILD.into())
            .unwrap()
            .consumption
            .is_contribution()
    );
    let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
    let mut request = invocation(&compiled, principal("user-a"), active.generation, "");
    request.capability_id = CHILD.into();
    request.resource_binding_ids = compiled
        .policy(&CHILD.into())
        .unwrap()
        .resource_binding_ids
        .clone();
    fixture
        .kernel
        .invoke(&compiled, &active, request)
        .await
        .unwrap();
}

#[test]
fn provider_induced_cycle_is_rejected_and_dependency_changes_invalidate_clean_save() {
    let mut fixture = Fixture::new();
    fixture.use_role_parent(true);
    let registry = fixture.kernel.snapshot().unwrap();
    let revision = current_revision(&fixture, &registry);
    let mut env = compiler_environment(registry.registry_digest.clone());
    env.host_surface = "test".into();
    assert!(AgentPresetCompiler::role_providers_unchanged(
        &registry,
        &env,
        &revision,
        &fixture.snapshot.envelope
    ));
    let mut changed = fixture.registrations.clone();
    let provider = changed.last_mut().unwrap();
    provider
        .metadata
        .manifest
        .payload
        .contributions
        .capabilities[0]
        .requires = vec![CapabilityRef {
        id: PEER.into(),
        version: VERSION.into(),
    }];
    refresh_manifest(provider);
    let registry = fixture.kernel.replace_all(changed.clone()).unwrap();
    assert!(!AgentPresetCompiler::role_providers_unchanged(
        &registry,
        &env,
        &revision,
        &fixture.snapshot.envelope
    ));
    let updated = AgentPresetCompiler::compile(
        &registry,
        &env,
        compile_request(revision.clone(), principal("user-a")),
    )
    .unwrap();
    assert!(updated.resolved_capability(&CHILD.into()).is_none());
    assert!(updated.resolved_capability(&PEER.into()).is_some());
    assert!(
        fixture
            .snapshot
            .resolved_capability(&CHILD.into())
            .is_some()
    );
    let provider = changed.last_mut().unwrap();
    provider
        .metadata
        .manifest
        .payload
        .contributions
        .capabilities[0]
        .requires = vec![CapabilityRef {
        id: SAMPLE_CAPABILITY.into(),
        version: VERSION.into(),
    }];
    refresh_manifest(provider);
    let registry = fixture.kernel.replace_all(changed).unwrap();
    assert!(matches!(
        AgentPresetCompiler::compile(
            &registry,
            &env,
            compile_request(revision, principal("user-a"))
        ),
        Err(KernelError::CapabilityDependencyCycle)
    ));
}

#[tokio::test]
async fn recursive_calls_keep_frozen_identity_and_each_dependency_resource_policy() {
    let fixture = Fixture::new();
    let before = fixture.snapshot.envelope.clone();
    let result = fixture
        .invoke(json!({"next":CHILD,"input":{"next":GRANDCHILD}}))
        .await
        .unwrap();
    assert_eq!(result.0["reached"], GRANDCHILD);
    let contexts = fixture.probe.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 3);
    let root = &contexts[0];
    let child = &contexts[1];
    let grandchild = &contexts[2];
    for context in contexts.iter() {
        assert_eq!(context.principal, root.principal);
        assert_eq!(context.agent_session_id, root.agent_session_id);
        assert_eq!(context.resolved_snapshot_ref, root.resolved_snapshot_ref);
        assert_eq!(context.correlation_id, root.correlation_id);
        assert_eq!(context.state_scope_key, root.state_scope_key);
    }
    assert_ne!(root.operation_id, child.operation_id);
    assert_ne!(root.idempotency_key, child.idempotency_key);
    assert_ne!(child.idempotency_key, grandchild.idempotency_key);
    assert_eq!(
        child.resource_bindings,
        vec![role_resource_binding("user-a")]
    );
    assert!(grandchild.resource_bindings.is_empty());
    assert_eq!(fixture.snapshot.envelope, before);
}

#[tokio::test]
async fn recursive_dependency_roles_use_the_same_default_and_override_resolver() {
    let mut fixture = Fixture::new();
    fixture.use_role_parent(true);
    let child = fixture.registrations[1]
        .metadata
        .manifest
        .payload
        .contributions
        .capabilities[0]
        .clone();
    let contract = RoleContractManifest {
        key: RoleContractKey {
            role_id: "sample.child.role".into(),
            contract_version: VERSION.into(),
        },
        members: vec![RoleMemberContract {
            capability: CapabilityRef {
                id: CHILD.into(),
                version: VERSION.into(),
            },
            capability_manifest_digest: digest_payload(&child).unwrap(),
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    let role = ExactRoleContractRef {
        key: contract.key.clone(),
        contract_digest: digest_payload(&contract).unwrap(),
    };
    let mut registration = PluginRegistration::new(fixture.registrations[1].metadata.clone());
    registration
        .metadata
        .manifest
        .payload
        .contributions
        .role_contracts = vec![contract];
    registration
        .metadata
        .manifest
        .payload
        .contributions
        .role_providers = vec![RoleProviderContribution {
        role: role.clone(),
        display: display("Child role", "Dependency-selected role"),
        members: BTreeMap::from([(
            CHILD.into(),
            RoleProviderMemberContribution {
                implementation: None,
                supported_platforms: vec![PlatformConstraint::Any],
                required_resource_kinds: child.contributions.resource_kinds.clone(),
            },
        )]),
    }];
    registration
        .add_role_action_handler(
            role.key.role_id.clone(),
            CHILD.into(),
            fixture.probe.clone(),
        )
        .unwrap();
    refresh_manifest(&mut registration);
    fixture.registrations[1] = registration;
    let registry = fixture
        .kernel
        .replace_all(fixture.registrations.clone())
        .unwrap();
    let mut revision = current_revision(&fixture, &registry);
    let mut env = compiler_environment(registry.registry_digest.clone());
    env.host_surface = "test".into();
    assert!(matches!(
        AgentPresetCompiler::compile(
            &registry,
            &env,
            compile_request(revision.clone(), principal("user-a"))
        ),
        Err(KernelError::RoleProviderNotBound { .. })
    ));
    let selection = nomifun_agent_contracts::RoleProviderSelection {
        role: role.clone(),
        provider_mount_id: CHILD.into(),
    };
    env.installation_role_bindings.insert(
        role.key.role_id.clone(),
        nomifun_agent_contracts::InstallationRoleBinding {
            selection: selection.clone(),
            binding_version: 1,
            updated_at_ms: 1,
        },
    );
    let compiled = AgentPresetCompiler::compile(
        &registry,
        &env,
        compile_request(revision.clone(), principal("user-a")),
    )
    .unwrap()
    .with_target_resource_bindings(
        &principal("user-a"),
        vec![resource_binding("user-a"), role_resource_binding("user-a")],
    )
    .unwrap();
    assert_eq!(compiled.content().resolved_role_providers.len(), 2);
    assert!(
        !compiled
            .resolved_capability(&CHILD.into())
            .unwrap()
            .consumption
            .is_contribution()
    );
    fixture.snapshot = compiled;
    fixture.active = SessionCapabilityState::new(&fixture.snapshot)
        .snapshot()
        .unwrap();
    assert_eq!(
        fixture
            .invoke(json!({"next":CHILD,"input":{"next":GRANDCHILD}}))
            .await
            .unwrap()
            .0["reached"],
        GRANDCHILD
    );
    let mut invalid_override = selection;
    invalid_override.provider_mount_id = "missing-provider".into();
    revision
        .payload
        .system_role_provider_overrides
        .insert(role.key.role_id, invalid_override);
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    assert!(matches!(
        AgentPresetCompiler::compile(
            &registry,
            &env,
            compile_request(revision, principal("user-a"))
        ),
        Err(KernelError::RoleProviderUnavailable { .. })
    ));
}

#[tokio::test]
async fn unrelated_authorized_and_transitive_only_targets_are_not_direct_dependencies() {
    let fixture = Fixture::new();
    assert!(fixture.snapshot.policy(&CapabilityId::from(PEER)).is_some());
    for target in [PEER, GRANDCHILD] {
        code(
            fixture.invoke(json!({"next":target})).await.unwrap_err(),
            "DEPENDENCY_NOT_DECLARED",
        );
    }
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn child_cannot_reenter_an_ancestor_or_reuse_an_effect_key() {
    let fixture = Fixture::new();
    code(
        fixture
            .invoke(json!({"next":CHILD,"input":{"next":SAMPLE_CAPABILITY}}))
            .await
            .unwrap_err(),
        "DEPENDENCY_CALL_CYCLE",
    );
    code(
        fixture
            .invoke(json!({"next":CHILD,"twice":true}))
            .await
            .unwrap_err(),
        "DEPENDENCY_CALL_KEY_REUSED",
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn child_actions_activation_and_resource_owner_are_enforced_before_dispatch() {
    let mut fixture = Fixture::new();
    fixture
        .snapshot
        .authority_policies
        .get_mut(&CapabilityId::from(CHILD))
        .unwrap()
        .allowed_actions
        .clear();
    code(
        fixture.invoke(json!({"next":CHILD})).await.unwrap_err(),
        nomifun_agent_contracts::CAPABILITY_NOT_IN_PRESET,
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);

    let mut fixture = Fixture::new();
    fixture.active.active.remove(&CapabilityId::from(CHILD));
    code(
        fixture.invoke(json!({"next":CHILD})).await.unwrap_err(),
        nomifun_agent_contracts::CAPABILITY_NOT_ACTIVE,
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);

    let mut fixture = Fixture::new();
    fixture
        .snapshot
        .target_resource_bindings
        .iter_mut()
        .find(|binding| binding.binding_id.as_ref() == SAMPLE_ROLE_BINDING)
        .unwrap()
        .owner_id = "other-user".into();
    code(
        fixture.invoke(json!({"next":CHILD})).await.unwrap_err(),
        nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH,
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn retained_callers_close_after_parent_success_and_failure() {
    let fixture = Fixture::new();
    fixture.invoke(json!({})).await.unwrap();
    code(
        fixture
            .caller(SAMPLE_CAPABILITY)
            .invoke(call(CHILD, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
    fixture.invoke(json!({"next":PEER})).await.unwrap_err();
    code(
        fixture
            .caller(SAMPLE_CAPABILITY)
            .invoke(call(CHILD, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn parent_drop_cancels_detached_managed_child_and_closes_its_caller() {
    let mut fixture = Fixture::new();
    let parent = fixture.spawn_parent();
    fixture.entered(SAMPLE_CAPABILITY).await;
    let caller = fixture.caller(SAMPLE_CAPABILITY);
    let child =
        tokio::spawn(async move { caller.invoke(call(CHILD, json!({"pause":true}))).await });
    fixture.entered(CHILD).await;
    code(
        fixture
            .caller(SAMPLE_CAPABILITY)
            .invoke(call(CHILD, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_CALL_KEY_REUSED",
    );
    parent.abort();
    assert!(parent.await.unwrap_err().is_cancelled());
    code(
        tokio::time::timeout(Duration::from_secs(3), child)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
    assert_eq!(fixture.probe.dropped.load(Ordering::SeqCst), 2);
    code(
        fixture
            .caller(CHILD)
            .invoke(call(GRANDCHILD, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
}

#[tokio::test]
async fn parent_and_child_source_drift_are_live_and_shared_by_registry_clones() {
    for id in [SAMPLE_CAPABILITY, CHILD] {
        let mut fixture = Fixture::new();
        let parent = fixture.spawn_parent();
        fixture.entered(SAMPLE_CAPABILITY).await;
        let index = fixture
            .registrations
            .iter()
            .position(|registration| {
                registration.metadata.manifest.payload.package_id.as_ref() == id
            })
            .unwrap();
        fixture.registrations[index].metadata.source.source_identity = format!("changed-{id}");
        fixture
            .kernel
            .clone()
            .replace_all(fixture.registrations.clone())
            .unwrap();
        let error = fixture
            .caller(SAMPLE_CAPABILITY)
            .invoke(call(CHILD, json!({})))
            .await
            .unwrap_err();
        assert!(
            matches!(error, KernelError::CapabilityProvenanceDrift { .. }),
            "{error:?}"
        );
        assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);
        parent.abort();
        let _ = parent.await;
    }
}

#[tokio::test]
async fn ancestor_revocation_is_checked_even_when_the_immediate_parent_is_unchanged() {
    let mut fixture = Fixture::new();
    let parent = fixture.spawn_parent();
    fixture.entered(SAMPLE_CAPABILITY).await;
    let caller = fixture.caller(SAMPLE_CAPABILITY);
    let child =
        tokio::spawn(async move { caller.invoke(call(CHILD, json!({"pause":true}))).await });
    fixture.entered(CHILD).await;
    fixture.registrations[0].metadata.source.source_identity = "root-withdrawn".into();
    fixture
        .kernel
        .replace_all(fixture.registrations.clone())
        .unwrap();
    let error = fixture
        .caller(CHILD)
        .invoke(call(GRANDCHILD, json!({})))
        .await
        .unwrap_err();
    assert!(
        matches!(error, KernelError::CapabilityProvenanceDrift { capability_id, .. } if capability_id.as_ref() == SAMPLE_CAPABILITY)
    );
    parent.abort();
    let _ = parent.await;
    code(
        child.await.unwrap().unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
}

#[tokio::test]
async fn explicit_outer_retries_get_stable_scoped_effect_ids_without_implicit_deduplication() {
    let fixture = Fixture::new();
    for _ in 0..2 {
        fixture.invoke(json!({"next":CHILD})).await.unwrap();
    }
    let contexts = fixture.probe.contexts.lock().unwrap();
    assert_eq!(contexts[1].idempotency_key, contexts[3].idempotency_key);
    assert_eq!(contexts[1].operation_id, contexts[3].operation_id);
    // The owning handler remains responsible for deduplication, exactly as on
    // the existing direct invocation path. No retry is started by the Kernel.
    assert_eq!(contexts.len(), 4);
}

#[tokio::test]
async fn selected_role_parents_use_the_same_dependency_dispatch_and_live_provider_lock() {
    for mapped in [false, true] {
        let mut fixture = Fixture::new();
        fixture.use_role_parent(mapped);
        fixture.invoke(json!({"next":CHILD})).await.unwrap();
        let context = fixture.probe.contexts.lock().unwrap()[0].clone();
        assert_eq!(
            context.role_provider.unwrap().provider.mount_id.as_ref(),
            "sample-provider"
        );
        assert_eq!(
            context.state.descriptor().mount_id.as_ref(),
            "sample-provider"
        );
        // Consume the notifications from the successful call first.
        fixture.entered(SAMPLE_CAPABILITY).await;
        fixture.entered(CHILD).await;
        let parent = fixture.spawn_parent();
        fixture.entered(SAMPLE_CAPABILITY).await;
        fixture
            .registrations
            .last_mut()
            .unwrap()
            .metadata
            .source
            .source_identity = "different-provider-source".into();
        fixture
            .kernel
            .replace_all(fixture.registrations.clone())
            .unwrap();
        let error = fixture
            .caller(SAMPLE_CAPABILITY)
            .invoke(call(CHILD, json!({})))
            .await
            .unwrap_err();
        assert!(
            matches!(error, KernelError::RoleProviderUnavailable { .. }),
            "{error:?}"
        );
        assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 3);
        parent.abort();
        let _ = parent.await;
    }
}

#[tokio::test]
async fn retained_callback_cannot_bind_to_a_later_parent_or_keep_its_registry_alive() {
    let mut fixture = Fixture::new();
    fixture.invoke(json!({})).await.unwrap();
    fixture.entered(SAMPLE_CAPABILITY).await;
    let retained = fixture.caller(SAMPLE_CAPABILITY);
    let parent = fixture.spawn_parent();
    fixture.entered(SAMPLE_CAPABILITY).await;
    code(
        retained.invoke(call(CHILD, json!({}))).await.unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
    fixture
        .caller(SAMPLE_CAPABILITY)
        .invoke(call(CHILD, json!({})))
        .await
        .unwrap();
    parent.abort();
    let _ = parent.await;
    let probe = Arc::downgrade(&fixture.probe);
    drop(fixture);
    assert!(
        probe.upgrade().is_none(),
        "retained callbacks must not form Registry -> handler -> callback cycles"
    );
    code(
        retained.invoke(call(CHILD, json!({}))).await.unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
}

#[tokio::test]
async fn local_effect_keys_are_bounded_and_namespaced_to_the_session() {
    let mut fixture = Fixture::new();
    let parent = fixture.spawn_parent();
    fixture.entered(SAMPLE_CAPABILITY).await;
    let caller = fixture.caller(SAMPLE_CAPABILITY);
    for key in [String::new(), " ".into(), "x".repeat(129)] {
        let mut request = call(CHILD, json!({}));
        request.call_key = key;
        code(
            caller.invoke(request).await.unwrap_err(),
            "DEPENDENCY_CALL_KEY_INVALID",
        );
    }
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);
    parent.abort();
    let _ = parent.await;
    fixture.invoke(json!({"next":CHILD})).await.unwrap();
    let mut request = fixture.request(json!({"next":CHILD}));
    request.agent_session_id = "another-agent-session".into();
    fixture
        .kernel
        .invoke(&fixture.snapshot, &fixture.active, request)
        .await
        .unwrap();
    let contexts = fixture.probe.contexts.lock().unwrap();
    assert_ne!(contexts[2].idempotency_key, contexts[4].idempotency_key);
}

#[tokio::test]
async fn dependency_call_bookkeeping_is_bounded_without_starting_over_limit_effects() {
    let mut fixture = Fixture::new();
    let parent = fixture.spawn_parent();
    fixture.entered(SAMPLE_CAPABILITY).await;
    let caller = fixture.caller(SAMPLE_CAPABILITY);
    for index in 0..1024 {
        let mut request = call(CHILD, json!({}));
        request.call_key = format!("effect-{index}");
        caller.invoke(request).await.unwrap();
    }
    let mut request = call(CHILD, json!({}));
    request.call_key = "over-capacity".into();
    code(
        caller.invoke(request).await.unwrap_err(),
        "DEPENDENCY_CALL_LIMIT",
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1025);
    parent.abort();
    let _ = parent.await;
}
