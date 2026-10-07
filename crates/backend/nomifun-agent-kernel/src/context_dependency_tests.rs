use super::*;
use crate::{
    CapabilityContextContributionFactory, CapabilityContextContributionRequest,
    ContextContributionResult,
};

#[derive(Default)]
struct ContextProbe {
    caller: Mutex<Option<CapabilityDependencyCaller>>,
    entered: Notify,
}

#[async_trait]
impl CapabilityContextContributionFactory for ContextProbe {
    async fn contribute(
        &self,
        request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        *self.caller.lock().unwrap() = Some(request.dependencies);
        self.entered.notify_one();
        std::future::pending().await
    }
}

fn context_fixture() -> (Fixture, Arc<ContextProbe>) {
    let mut fixture = Fixture::new();
    let probe = Arc::new(ContextProbe::default());
    let mut root = PluginRegistration::new(fixture.registrations[0].metadata.clone());
    let capability = &mut root.metadata.manifest.payload.contributions.capabilities[0];
    capability.kind = CapabilityKind::ContextContributor;
    capability.contributions.context_schema_refs =
        vec![capability.contributions.actions[0].output_schema.clone()];
    capability.contributions.actions.clear();
    refresh_manifest(&mut root);
    root.add_capability_context_factory(SAMPLE_CAPABILITY.into(), probe.clone())
        .unwrap();
    fixture.registrations[0] = root;
    let registry = fixture
        .kernel
        .replace_all(fixture.registrations.clone())
        .unwrap();
    let mut revision = current_revision(&fixture, &registry);
    revision
        .payload
        .enabled_capabilities
        .iter_mut()
        .find(|value| value.capability.id.as_ref() == SAMPLE_CAPABILITY)
        .unwrap()
        .action_allowlist
        .clear();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    fixture.snapshot = AgentPresetCompiler::compile(
        &registry,
        &compiler_environment(registry.registry_digest.clone()),
        compile_request(revision, principal("user-a")),
    )
    .unwrap()
    .with_target_resource_bindings(
        &principal("user-a"),
        vec![resource_binding("user-a"), role_resource_binding("user-a")],
    )
    .unwrap();
    fixture.active = SessionCapabilityState::new(&fixture.snapshot)
        .snapshot()
        .unwrap();
    (fixture, probe)
}

async fn start_context(
    fixture: &Fixture,
    probe: &ContextProbe,
) -> tokio::task::JoinHandle<Result<ContextContributionResult, KernelError>> {
    let request =
        crate::dependency_call::DependencyAncestor::Tool(fixture.request(json!({}))).access();
    let (kernel, snapshot, active) = (
        fixture.kernel.clone(),
        fixture.snapshot.clone(),
        fixture.active.clone(),
    );
    let parent =
        tokio::spawn(async move { kernel.contribute_context(&snapshot, &active, request).await });
    tokio::time::timeout(Duration::from_secs(3), probe.entered.notified())
        .await
        .unwrap();
    parent
}

fn caller(probe: &ContextProbe) -> CapabilityDependencyCaller {
    probe.caller.lock().unwrap().as_ref().unwrap().clone()
}

#[tokio::test]
async fn context_children_keep_identity_and_child_resource_policy_without_a_parent_action() {
    let (fixture, probe) = context_fixture();
    let parent = start_context(&fixture, &probe).await;
    assert!(
        fixture
            .snapshot
            .policy(&SAMPLE_CAPABILITY.into())
            .unwrap()
            .allowed_actions
            .is_empty()
    );
    let caller = caller(&probe);
    caller
        .invoke(call(CHILD, json!({"next":GRANDCHILD})))
        .await
        .unwrap();
    {
        let contexts = fixture.probe.contexts.lock().unwrap();
        assert_eq!(contexts.len(), 2);
        let request = fixture.request(json!({}));
        for child in contexts.iter() {
            assert_eq!(child.principal, request.principal);
            assert_eq!(child.agent_session_id, request.agent_session_id);
            assert_eq!(child.resolved_snapshot_ref, request.resolved_snapshot_ref);
            assert_eq!(child.correlation_id, request.correlation_id);
            assert_eq!(child.state_scope_key, request.state_scope_key);
        }
        assert_eq!(
            contexts[0].resource_bindings,
            vec![role_resource_binding("user-a")]
        );
        assert!(contexts[1].resource_bindings.is_empty());
        assert_ne!(contexts[0].idempotency_key, contexts[1].idempotency_key);
    }
    code(
        caller.invoke(call(CHILD, json!({}))).await.unwrap_err(),
        "DEPENDENCY_CALL_KEY_REUSED",
    );
    for target in [PEER, GRANDCHILD] {
        code(
            caller.invoke(call(target, json!({}))).await.unwrap_err(),
            "DEPENDENCY_NOT_DECLARED",
        );
    }
    code(
        caller
            .invoke(call(SAMPLE_CAPABILITY, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_CALL_CYCLE",
    );
    parent.abort();
    assert!(parent.await.unwrap_err().is_cancelled());
    code(
        caller.invoke(call(CHILD, json!({}))).await.unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
}

#[tokio::test]
async fn context_child_authority_is_not_implied_by_dependency_membership() {
    for scenario in 0..3 {
        let (mut fixture, probe) = context_fixture();
        let expected = match scenario {
            0 => {
                fixture
                    .snapshot
                    .authority_policies
                    .get_mut(&CHILD.into())
                    .unwrap()
                    .allowed_actions
                    .clear();
                nomifun_agent_contracts::CAPABILITY_NOT_IN_PRESET
            }
            1 => {
                fixture.active.active.remove(&CHILD.into());
                nomifun_agent_contracts::CAPABILITY_NOT_ACTIVE
            }
            _ => {
                fixture
                    .snapshot
                    .target_resource_bindings
                    .iter_mut()
                    .find(|value| value.binding_id.as_ref() == SAMPLE_ROLE_BINDING)
                    .unwrap()
                    .owner_id = "other-user".into();
                nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH
            }
        };
        let parent = start_context(&fixture, &probe).await;
        code(
            caller(&probe)
                .invoke(call(CHILD, json!({})))
                .await
                .unwrap_err(),
            expected,
        );
        assert!(fixture.probe.contexts.lock().unwrap().is_empty());
        parent.abort();
        assert!(parent.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn context_ancestor_revocation_and_drop_apply_to_running_descendants() {
    let (mut fixture, probe) = context_fixture();
    let parent = start_context(&fixture, &probe).await;
    let caller = caller(&probe);
    let child =
        tokio::spawn(async move { caller.invoke(call(CHILD, json!({"pause":true}))).await });
    fixture.entered(CHILD).await;
    let mut changed = fixture.registrations.clone();
    changed[0].metadata.source.source_digest = Some(DigestHex::from("f".repeat(64)));
    fixture.kernel.replace_all(changed).unwrap();
    assert!(
        fixture
            .caller(CHILD)
            .invoke(call(GRANDCHILD, json!({})))
            .await
            .is_err()
    );
    assert_eq!(fixture.probe.contexts.lock().unwrap().len(), 1);
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
    assert_eq!(fixture.probe.dropped.load(Ordering::SeqCst), 1);
    code(
        fixture
            .caller(CHILD)
            .invoke(call(GRANDCHILD, json!({})))
            .await
            .unwrap_err(),
        "DEPENDENCY_PARENT_CLOSED",
    );
}
