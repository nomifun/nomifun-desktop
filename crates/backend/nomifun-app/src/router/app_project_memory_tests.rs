//! Exercise production workspace freezing, ordinary canonical Session opening,
//! Kernel admission and the real project-memory host/state owner. There is no
//! model, mock memory port, direct state write or fabricated success receipt.
use super::*;
use nomifun_agent_contracts::{AgentPresetRevision, AgentPresetRevisionPayload, CapabilityRef,
    CapabilitySelection, PresetRevisionRef};
use nomifun_agent_kernel::{AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest,
    CompiledSnapshot, CompilerEnvironment, InMemoryPluginStatePersistence, KernelRegistry,
    MaterializationPolicy, SessionCapabilityState};

#[tokio::test]
async fn new_session_after_verified_application_relocation_reuses_current_logical_memory() {
    let fixture = super::app_completion_fence_tests::Fixture::new().await;
    let target = tempfile::tempdir().unwrap();
    let target_root = nomifun_common::paths::canonicalize_simplified(target.path()).unwrap();
    let project = target_root.join("app-hosting").join("apps").join(&fixture.app.id);
    std::fs::create_dir_all(&project).unwrap();
    let store = fixture.state.session_owner.canonical().store();
    let old = store.get_live_session(&fixture.session).await.unwrap();
    let old_memory = old.agent_binding.typed_resource_bindings.iter()
        .find(|resource| resource.resource_kind.as_ref() == "project_memory").unwrap().resource_id.clone();
    assert!(old_memory.as_ref().starts_with(PROJECT_WORKSPACE_MEMORY_PREFIX));
    // The production location coordinator resolves existing source roots before
    // this Store call. Windows TEMP can be a DOS 8.3 alias; use the same exact
    // canonical source identity as the frozen project instead of its raw config.
    std::fs::create_dir_all(&fixture.services.work_dir).unwrap();
    let source_data = nomifun_common::paths::canonicalize_simplified(&fixture.services.data_dir).unwrap();
    let source_work = nomifun_common::paths::canonicalize_simplified(&fixture.services.work_dir).unwrap();
    let relocation = store.relocate_managed_workspaces_with_app_workspaces(&source_data, &source_work,
        &target_root, &OperationId::from(uuid::Uuid::now_v7().to_string()), &BTreeSet::from([fixture.app.id.clone()])).await.unwrap();
    assert!(relocation.app_workspaces.contains_key(&fixture.session));
    let moved = store.get_live_session(&fixture.session).await.unwrap();
    assert!(moved.agent_binding.typed_resource_bindings.iter().filter(|resource|
        matches!(resource.resource_kind.as_ref(), "workspace" | "process_session" | "project_memory"))
        .all(|resource| resource.typed_parameters.get("workspace_root").is_some_and(|root| root == project.to_str().unwrap())));
    let owner = AuthenticatedOwner(UserId::from(fixture.principal.principal_id.clone()));
    let Json(created) = create_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner), HeaderMap::new(),
        Json(CreateAgentSessionRequestDto { preset_id: old.agent_binding.preset_revision_ref.preset_id.as_ref().to_owned(),
            title: Some("Next project iteration".into()), model: None, reasoning_effort: None,
            resource_selections: Vec::new(), knowledge_policy: None, workspace: Some(project.to_string_lossy().into_owned()),
            session_capabilities: Some(Default::default()), required_modules: BTreeSet::new() })).await.unwrap();
    let next = store.get_live_session(&parse_agent_session_id(&created.data.unwrap().agent_session_id).unwrap()).await.unwrap();
    assert_eq!(next.agent_binding.typed_resource_bindings.iter().find(|resource| resource.resource_kind.as_ref() == "project_memory").unwrap().resource_id, old_memory);
    let (_, revision, _) = fixture.state.control_plane.saved_binding_artifacts(
        &UserId::from(fixture.principal.principal_id.clone()), &agent_binding_dto(&moved.agent_binding).unwrap(),
    ).await.unwrap();
    let route = &revision.payload.chat_route_records[nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT];
    let same_model = AgentChatModelSelectionDto { provider_id:route.primary.provider_id.clone(), model:route.primary.model.clone() };
    let scheduled = fixture.state.product_agent_resolver.prepare_new_binding(
        &fixture.principal.principal_id,
        &nomifun_api_types::ProductAgentSelectionDto::Template { template_key:"app.developer".into() },
        Some(&same_model), Some(project.to_str().unwrap()), &[],
    ).await.unwrap();
    assert_eq!(scheduled.typed_resource_bindings.iter().find(|resource| resource.resource_kind == "project_memory").unwrap().resource_id,
        old_memory.as_ref(), "automatic preparation reuses the same verified project namespace as interactive creation");
    let revised = fixture.state.product_agent_resolver.revise_binding(&fixture.principal.principal_id,
        &scheduled, Some(&same_model), None, None).await.unwrap();
    assert_eq!(revised, scheduled, "a model-only no-op preserves every exact resource definition");
    let cleared = fixture.state.product_agent_resolver.revise_binding(&fixture.principal.principal_id,
        &scheduled, None, Some(""), None).await.unwrap();
    assert_eq!(cleared.preset_revision_ref, scheduled.preset_revision_ref);
    assert_eq!(cleared.resolved_snapshot_ref, scheduled.resolved_snapshot_ref);
    assert!(cleared.typed_resource_bindings.iter().filter(|resource| matches!(resource.resource_kind.as_str(), "workspace" | "process_session"))
        .all(|resource| resource.typed_parameters == nomifun_agent_contracts::managed_workspace_parameters()));
    assert_eq!(cleared.typed_resource_bindings.iter().find(|resource| resource.resource_kind == "project_memory").unwrap().resource_id,
        super::super::nomi_core_resource_bindings::DEFAULT_PROJECT_MEMORY_RESOURCE_ID);
    assert_eq!(store.get_live_session(&fixture.session).await.unwrap(), moved, "new-session admission cannot rewrite the relocated Session");
    fixture.finish().await;
}

fn revision(owner: &PrincipalRef) -> AgentPresetRevision {
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef { preset_id:"project-memory-fixture".into(), revision:1, revision_digest:"".into() },
        payload: AgentPresetRevisionPayload {
            context_order:Vec::new(), middleware_order:Vec::new(), schema_version:"1.0.0".into(),
            model_route_refs:BTreeMap::new(), chat_route_records:BTreeMap::new(),
            enabled_capabilities:vec![CapabilitySelection { capability:CapabilityRef { id:"project.memory".into() },
                action_allowlist:BTreeSet::from(["project.memory/read".into(),"project.memory/write".into()]) }],
            skill_bindings:Vec::new(), system_role_provider_overrides:BTreeMap::new(), persona:"Memory domain fixture".into(),
            instructions:String::new(), starter_prompts:Vec::new(), runtime_policy:Default::default(),
        },
        contribution_locks:Vec::new(), created_by:owner.principal_id.clone().into(), created_at_ms:1, reason:None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

fn project_binding(owner: &PrincipalRef, revision: &AgentPresetRevision, snapshot: &CompiledSnapshot,
    directory: &std::path::Path) -> AgentBindingValue {
    let mut binding = AgentBindingValueDto {
        preset_revision_ref:serde_json::from_value(serde_json::to_value(&revision.reference).unwrap()).unwrap(),
        resolved_snapshot_ref:serde_json::from_value(serde_json::to_value(snapshot.snapshot_ref()).unwrap()).unwrap(),
        binding_version:1,
        typed_resource_bindings:vec![TypedResourceBindingDto {
            binding_id:"default-memory-fixture".into(), resource_kind:"project_memory".into(), resource_id:"default-project-memory".into(),
            owner_id:owner.principal_id.clone(), operations:BTreeSet::from(["read".into(),"write".into()]),
            connection_config_ref:None, typed_parameters:BTreeMap::new(),
        }],
    };
    freeze_selected_workspace(&mut binding, &owner.principal_id, &directory.to_string_lossy(), WorkspaceDirectoryCheck::Create).unwrap();
    serde_json::from_value(serde_json::to_value(binding).unwrap()).unwrap()
}

async fn invoke_memory(kernel: &KernelRegistry, snapshot: &CompiledSnapshot, owner: &PrincipalRef,
    session: &AgentSessionId, action: &str, key: &str, input: Value) -> Value {
    let active = SessionCapabilityState::new(snapshot).snapshot().unwrap();
    let result = kernel.invoke(snapshot, &active, CapabilityInvocationRequest {
        principal:owner.clone(), session_owner:owner.clone(), agent_session_id:session.clone(),
        turn_id:format!("domain-turn:{key}").into(), operation_id:format!("domain-operation:{key}").into(),
        idempotency_key:key.to_owned().into(), correlation_id:format!("domain-correlation:{key}").into(),
        resolved_snapshot_ref:snapshot.snapshot_ref().clone(), active_set_generation:active.generation,
        capability_id:"project.memory".into(), action_id:action.to_owned().into(),
        resource_binding_ids:snapshot.policy(&"project.memory".into()).unwrap().resource_binding_ids.clone(),
        state_scope_key:format!("session:{}", session.as_ref()).into(), input:nomifun_agent_contracts::StrictJsonValue(input),
    }).await.unwrap();
    result.0
}

#[tokio::test]
async fn project_memory_real_kernel_host_shares_between_sessions_and_isolates_other_projects() {
    let temporary = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = crate::services::AppServices::from_config(database, &crate::AppConfig {
        data_dir:temporary.path().join("data"), work_dir:temporary.path().join("work"), ..Default::default()
    }).await.unwrap();
    let owner = PrincipalRef { principal_kind:"user".into(), principal_id:services.authoritative_user_id.as_ref().to_owned() };
    let app = services.app_hosting.create("First memory project".into(), Default::default()).await.unwrap();
    let other = services.app_hosting.create("Other memory project".into(), Default::default()).await.unwrap();
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let kernel = KernelRegistry::new(MaterializationPolicy::stable("1.0.0"), persistence.clone()).unwrap();
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(services.database.pool().clone()).await.unwrap();
    let materialized = kernel.replace_all(super::super::agent_wave1_host::wave1_registrations_for_nomi_core(
        services.knowledge_service.clone(), None, services.companion_service.clone(), services.database.pool().clone(), canonical.store().clone()).unwrap()).unwrap();
    let mut revision = revision(&owner);
    revision.contribution_locks = vec![materialized.capability(&"project.memory".into()).unwrap().contribution_lock.clone()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let snapshot = AgentPresetCompiler::compile(&materialized, &CompilerEnvironment {
        resolver_version:"1.0.0".into(), required_runtime_protocol_version:"1.0.0".into(),
        required_runtime_profile:nomifun_agent_contracts::RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest:"a".repeat(64).into(), available_runtime_features:BTreeSet::new(),
        installation_role_bindings:BTreeMap::new(), canonical_schema_manifest_digest:"b".repeat(64).into(),
        target_contribution_manifest_digest:materialized.registry_digest.clone(), host_target:"windows-desktop-x64".into(),
        host_surface:"desktop".into(), availability_evidence_revision:"memory-domain-test".into(),
    }, CompileRequest { revision:revision.clone(), principal:owner.clone(), scene:"project".into(),
        surface:"desktop".into(), audience:"user".into(), created_at_ms:2, resolver_run_id:"resolve-project-memory-fixture".into() }).unwrap();
    let first_binding = project_binding(&owner, &revision, &snapshot, &app.directory);
    let same_binding = project_binding(&owner, &revision, &snapshot, &app.directory);
    let other_binding = project_binding(&owner, &revision, &snapshot, &other.directory);
    let memory_id = |binding: &AgentBindingValue| binding.typed_resource_bindings.iter()
        .find(|resource| resource.resource_kind.as_ref() == "project_memory").unwrap().resource_id.clone();
    assert_eq!(memory_id(&first_binding), memory_id(&same_binding));
    assert_ne!(memory_id(&first_binding), memory_id(&other_binding));
    let mut sessions = Vec::new();
    for (index, binding) in [&first_binding, &same_binding, &other_binding].into_iter().enumerate() {
        sessions.push(canonical.open(owner.clone(), binding.clone(), Some(format!("Memory Session {index}")),
            vec!["project.memory".into()], &format!("memory-session-{index}"), 10).await.unwrap().session.agent_session_id);
    }
    assert_ne!(sessions[0], sessions[1]);
    assert_eq!(canonical.store().list_live_sessions_for_workspace(&owner, None, 10,
        app.directory.to_str().unwrap()).await.unwrap().total, 2);
    let first = snapshot.clone().with_target_resource_bindings(&owner, first_binding.typed_resource_bindings.clone()).unwrap();
    let same = snapshot.clone().with_target_resource_bindings(&owner, same_binding.typed_resource_bindings.clone()).unwrap();
    let other_snapshot = snapshot.clone().with_target_resource_bindings(&owner, other_binding.typed_resource_bindings.clone()).unwrap();
    let written = invoke_memory(&kernel, &first, &owner, &sessions[0], "project.memory/write", "first-project-write",
        json!({"content":"First project: use the blue theme","title":"Theme decision"})).await;
    assert_eq!(written["persisted"], true);
    let read = invoke_memory(&kernel, &same, &owner, &sessions[1], "project.memory/read", "same-project-read", json!({})).await;
    assert_eq!(read["entries"].as_array().unwrap().len(), 1);
    assert_eq!(read["entries"][0]["request"]["content"], "First project: use the blue theme");
    let isolated = invoke_memory(&kernel, &other_snapshot, &owner, &sessions[2], "project.memory/read", "other-project-read", json!({})).await;
    assert!(isolated["entries"].as_array().unwrap().is_empty());
    invoke_memory(&kernel, &other_snapshot, &owner, &sessions[2], "project.memory/write", "other-project-write",
        json!({"content":"Other project: use the green theme"})).await;
    let first_again = invoke_memory(&kernel, &same, &owner, &sessions[1], "project.memory/read", "first-project-read-again", json!({})).await;
    assert_eq!(first_again["entries"].as_array().unwrap().len(), 1);
    assert!(!first_again.to_string().contains("green"));
    // Actual state entries are the results of real host CAS operations. The
    // established persistence here deliberately lasts one Host lifetime.
    let persisted = persistence.snapshot().unwrap();
    assert_eq!(persisted.entries().count(), 2);
    assert!(persisted.entries().all(|(identity, _)| identity.namespace().scope_key.as_ref()
        .starts_with(&format!("resource:{PROJECT_WORKSPACE_MEMORY_PREFIX}"))));
    services.cleanup_after_startup_failure(anyhow::anyhow!("project memory domain fixture finished")).await;
}
