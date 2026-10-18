use super::*;

fn process_authority() -> super::super::engine_process_host::ProcessAuthority {
    super::super::engine_process_host::ProcessAuthority {
        snapshot: nomifun_agent_contracts::ResolvedSnapshotRef {
            snapshot_id: "snapshot".into(),
            snapshot_digest: "b".repeat(64).into(),
        },
        actions: BTreeSet::from(["workspace.process/start".into()]),
        resources: Vec::new(),
    }
}

#[tokio::test]
async fn session_process_owner_accepts_first_turn_and_preserves_workspace_authority() {
    let workspace = tempfile::tempdir().unwrap();
    let other_workspace = tempfile::tempdir().unwrap();
    let (journal, pool) = super::super::engine_journal::test_fixture().await;
    let host = NomiCoreWave2Host::default();
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let session = "0190f5fe-7c00-7a00-8000-000000000002";
    let key = (owner.to_owned(), session.to_owned());
    let authority = process_authority();
    let root = canonical_workspace_root(workspace.path()).unwrap();
    let native_root = std::fs::canonicalize(workspace.path()).unwrap();
    #[cfg(windows)]
    if matches!(root.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_))) {
        assert_ne!(root, native_root, "exercise ordinary and verbatim Windows paths");
    }

    // This must also succeed for a newly created owner, before any process exists.
    host.open_runtime_turn(
        owner, session, "first", root.to_str().unwrap(), journal.clone(), authority.clone(),
    ).unwrap();
    let scope = host.processes.lock().unwrap().get(&key).unwrap().clone();
    let generation = scope.owner_generation().to_owned();
    host.finish_runtime_process_turn(owner, session, "first").unwrap();

    host.open_runtime_turn(
        owner, session, "second", native_root.to_str().unwrap(), journal.clone(), authority.clone(),
    ).unwrap();
    assert_eq!(
        host.processes.lock().unwrap().get(&key).unwrap().owner_generation(),
        generation,
    );
    host.finish_runtime_process_turn(owner, session, "second").unwrap();

    let rejected = host.open_runtime_turn(
        owner, session, "wrong-workspace", other_workspace.path().to_str().unwrap(),
        journal.clone(), authority.clone(),
    ).unwrap_err();
    assert!(matches!(rejected, AppError::Conflict(message)
        if message == "Session process workspace authority changed"));

    // Rejecting another directory must leave the original owner usable.
    host.open_runtime_turn(
        owner, session, "third", root.to_str().unwrap(), journal.clone(), authority,
    ).unwrap();
    assert_eq!(
        host.processes.lock().unwrap().get(&key).unwrap().owner_generation(),
        generation,
    );
    host.finish_runtime_process_turn(owner, session, "third").unwrap();
    host.cleanup_runtime_session(owner, session).await.unwrap();
    drop(scope);
    drop(host);
    drop(journal);
    pool.close().await;
}

#[tokio::test]
async fn session_process_owner_accepts_a_missing_default_directory_anchor() {
    let fixture = tempfile::tempdir().unwrap();
    let missing_default = fixture.path().join("missing-default");
    let (journal, pool) = super::super::engine_journal::test_fixture().await;
    let host = NomiCoreWave2Host::default();
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let session = "0190f5fe-7c00-7a00-8000-000000000002";

    host.open_runtime_turn(
        owner, session, "turn", missing_default.to_str().unwrap(),
        journal.clone(), process_authority(),
    ).unwrap();
    assert!(!missing_default.exists(), "opening authority must not materialize its anchor");
    host.finish_runtime_process_turn(owner, session, "turn").unwrap();
    host.cleanup_runtime_session(owner, session).await.unwrap();
    drop(host);
    drop(journal);
    pool.close().await;
}

struct ProcessContextCapture(Arc<Mutex<Option<Wave2HostContext>>>);

impl Wave2HostPort for ProcessContextCapture {
    fn invoke<'a>(&'a self, request: Wave2HostRequest) -> std::pin::Pin<Box<
        dyn std::future::Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a,
    >> {
        *self.0.lock().unwrap() = Some(request.context);
        Box::pin(std::future::ready(Ok(StrictJsonValue(json!({})))))
    }
}

// Obtain the opaque state handle through genuine Kernel authorization.
async fn process_poll_context(root: &Path) -> Wave2HostContext {
    use nomifun_agent_contracts::{
        ActionId, AgentPresetRevision, AgentPresetRevisionPayload, CapabilityRef,
        CapabilitySelection, RuntimeProfileKind,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest,
        CompilerEnvironment, InMemoryPluginStatePersistence, KernelRegistry,
        MaterializationPolicy, SessionCapabilityState,
    };

    let captured = Arc::new(Mutex::new(None));
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable(nomifun_agent_domain_wave2::CONTRACT_VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    ).unwrap();
    let materialized = registry.replace_all(
        nomifun_agent_domain_wave2::registrations_with_host_port(Arc::new(
            ProcessContextCapture(captured.clone()),
        )).unwrap(),
    ).unwrap();
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
    };
    let action = ActionId::from("workspace.process/poll");
    let binding = TypedResourceBinding {
        binding_id: "process-binding".into(),
        resource_kind: "process_session".into(),
        resource_id: "managed-process-session".into(),
        owner_id: principal.principal_id.clone(),
        operations: BTreeSet::from(["execute".into()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([(
            WORKSPACE_ROOT_PARAMETER.into(), root.to_str().unwrap().into(),
        )]),
    };
    let mut revision = AgentPresetRevision {
        reference: nomifun_agent_contracts::PresetRevisionRef {
            preset_id: "process-test".into(), revision: 1, revision_digest: "".into(),
        },
        payload: AgentPresetRevisionPayload {
            context_order: Vec::new(), middleware_order: Vec::new(),
            schema_version: nomifun_agent_domain_wave2::CONTRACT_VERSION.into(),
            model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef { id: WORKSPACE_PROCESS.into() },
                action_allowlist: BTreeSet::from([action.clone()]),
            }],
            skill_bindings: Vec::new(), system_role_provider_overrides: BTreeMap::new(),
            persona: "Process authority test".into(), instructions: "Poll an owned process.".into(),
            starter_prompts: Vec::new(), runtime_policy: Default::default(),
        },
        contribution_locks: vec![materialized.capability(&WORKSPACE_PROCESS.into())
            .unwrap().contribution_lock.clone()],
        created_by: principal.principal_id.clone().into(), created_at_ms: 1, reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let snapshot = AgentPresetCompiler::compile(
        &materialized,
        &CompilerEnvironment {
            resolver_version: nomifun_agent_domain_wave2::CONTRACT_VERSION.into(),
            required_runtime_protocol_version: nomifun_agent_domain_wave2::CONTRACT_VERSION.into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "runtime".into(),
            available_runtime_features: BTreeSet::new(), installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: "schema".into(),
            target_contribution_manifest_digest: "target".into(), host_target: "test-target".into(),
            host_surface: "desktop".into(), availability_evidence_revision: "process-test".into(),
        },
        CompileRequest {
            revision, principal: principal.clone(), scene: "process-test".into(),
            surface: "desktop".into(), audience: "test".into(), created_at_ms: 2,
            resolver_run_id: "resolve".into(),
        },
    ).unwrap().with_target_resource_bindings(&principal, vec![binding]).unwrap();
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    registry.invoke(&snapshot, &active, CapabilityInvocationRequest {
        principal: principal.clone(), session_owner: principal,
        agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
        turn_id: "turn".into(), operation_id: "poll".into(),
        idempotency_key: "poll".into(), correlation_id: "poll".into(),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(), active_set_generation: active.generation,
        capability_id: WORKSPACE_PROCESS.into(), action_id: action,
        resource_binding_ids: BTreeSet::from(["process-binding".into()]),
        state_scope_key: "session:process-test".into(),
        input: StrictJsonValue(json!({"process_id":"not-owned-by-this-session","wait_ms":0})),
    }).await.unwrap();
    let context = captured.lock().unwrap().take().unwrap();
    context
}

#[tokio::test]
async fn session_process_invoke_accepts_equivalent_paths_but_rejects_another_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let other_workspace = tempfile::tempdir().unwrap();
    let root = canonical_workspace_root(workspace.path()).unwrap();
    let other_root = canonical_workspace_root(other_workspace.path()).unwrap();
    let (journal, pool) = super::super::engine_journal::test_fixture().await;
    let host = NomiCoreWave2Host::default().with_effect_store(journal.canonical_store());
    let mut context = process_poll_context(&root).await;
    let authority = |context: &Wave2HostContext| super::super::engine_process_host::ProcessAuthority {
        snapshot: context.resolved_snapshot_ref.clone(),
        actions: BTreeSet::from([context.action_id.clone()]),
        resources: context.resource_bindings.clone(),
    };
    let request = |context: Wave2HostContext| Wave2HostRequest {
        context,
        operation: nomifun_agent_domain_wave2::Wave2CapabilityOperation::WorkspaceExecution {
            input: StrictJsonValue(json!({"process_id":"not-owned-by-this-session","wait_ms":0})),
        },
    };
    host.open_runtime_turn(
        &context.principal.principal_id, context.agent_session_id.as_ref(), context.turn_id.as_ref(),
        root.to_str().unwrap(), journal.clone(), authority(&context),
    ).unwrap();
    let observation = host.invoke(request(context.clone())).await.unwrap();
    assert_eq!(observation.0["code"], "PROCESS_REFERENCE_INVALID");
    assert_eq!(observation.0["state"], "not_executed");
    assert_eq!(observation.0["control_applied"], false);
    host.finish_runtime_process_turn(
        &context.principal.principal_id, context.agent_session_id.as_ref(), context.turn_id.as_ref(),
    ).unwrap();

    context.turn_id = "other-workspace-turn".into();
    context.resource_bindings[0].typed_parameters.insert(
        WORKSPACE_ROOT_PARAMETER.into(), other_root.to_str().unwrap().into(),
    );
    // Freeze binding B through normal admission while retaining the Session's
    // root A: authorization passes, so the workspace guard must catch the mismatch.
    host.open_runtime_turn(
        &context.principal.principal_id, context.agent_session_id.as_ref(), context.turn_id.as_ref(),
        root.to_str().unwrap(), journal.clone(), authority(&context),
    ).unwrap();
    let scope = host.processes.lock().unwrap().get(&(
        context.principal.principal_id.clone(), context.agent_session_id.as_ref().to_owned(),
    )).unwrap().clone();
    scope.authorize(&context).unwrap();
    let rejected = host.invoke(request(context.clone())).await.unwrap_err();
    assert_eq!(rejected.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(rejected.message, "process workspace authority changed");
    host.finish_runtime_process_turn(
        &context.principal.principal_id, context.agent_session_id.as_ref(), context.turn_id.as_ref(),
    ).unwrap();
    host.cleanup_runtime_session(&context.principal.principal_id, context.agent_session_id.as_ref()).await.unwrap();
    drop(scope);
    drop(host);
    drop(journal);
    pool.close().await;
}
