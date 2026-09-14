//! Shared domain-owner tests, independent of the retired Wrapper host.
use super::*;
use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, AgentSessionId, CapabilityId,
    CapabilityRef, CapabilitySelection, CorrelationId, DigestHex, IdempotencyKey, OperationId,
    PluginStateEntry, PresetRevisionRef, ResourceBindingId, ResourceId, ResourceKind,
    RuntimeProfileKind, RuntimeTarget, ScopeKey, StateKey, StrictJsonValue, TypedResourceBinding,
    UserId, VersionString,
};
use nomifun_agent_kernel::{
    ActiveCapabilitySetSnapshot, AgentPresetCompiler, CapabilityAccessRequest,
    CapabilityInvocationRequest, CompileRequest, CompiledSnapshot, CompilerEnvironment,
    InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, PluginStatePersistence,
    PluginStateSnapshot, SessionCapabilityState, StateIdentity,
};
use nomifun_common::KnowledgeBaseId;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
const CONTRACT_VERSION: &str = "1.0.0";

fn principal() -> nomifun_agent_contracts::PrincipalRef {
    nomifun_agent_contracts::PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: "0199a000-0000-7000-8000-000000000000".to_owned(),
    }
}

struct NoopCompanionCompleter;

#[async_trait]
impl nomifun_companion::learner::CompanionCompleter for NoopCompanionCompleter {
    async fn complete(
        &self,
        _provider_id: &str,
        _model: &str,
        _system: &str,
        _user: &str,
        _max_tokens: u32,
    ) -> Result<String, nomifun_common::AppError> {
        Ok("{}".to_owned())
    }
}

async fn companion_service(data_dir: &Path) -> Arc<nomifun_companion::CompanionService> {
    nomifun_companion::CompanionService::start(
        data_dir,
        Arc::new(nomifun_realtime::BroadcastEventBus::new(16)),
        &principal().principal_id,
        Arc::new(NoopCompanionCompleter),
        Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(
            data_dir, data_dir,
        )),
    )
    .await
    .expect("Companion service")
}

struct KnowledgeKernelFixture {
    registry: Arc<KernelRegistry>,
}

impl KnowledgeKernelFixture {
    fn new() -> Self {
        let registry = Arc::new(
            KernelRegistry::new(
                MaterializationPolicy::stable(CONTRACT_VERSION),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .expect("kernel registry"),
        );
        registry
            .replace_all(
                nomifun_agent_domain_wave1::registrations_with_host_port(Arc::new(
                    Wave1ApplicationHost::default(),
                ))
                .expect("Wave 1 registrations"),
            )
            .expect("publish Wave 1 registrations");
        Self { registry }
    }

    fn compile_snapshot(
        &self,
        binding: TypedResourceBinding,
    ) -> (
        Arc<CompiledSnapshot>,
        ActiveCapabilitySetSnapshot,
        TypedResourceBinding,
    ) {
        compile_wave1_snapshot_for_registry(
            &self.registry,
            &[
                nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
                nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            ],
            binding,
            "knowledge",
        )
    }

    async fn invoke(
        &self,
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        binding: &TypedResourceBinding,
        capability_id: &str,
        input: serde_json::Value,
        request_id: &str,
    ) -> Result<StrictJsonValue, nomifun_agent_kernel::KernelError> {
        self.registry
            .invoke(
                snapshot,
                active,
                knowledge_invocation(snapshot, active, binding, capability_id, input, request_id),
            )
            .await
    }
}

fn knowledge_binding(knowledge_base_id: &KnowledgeBaseId, root: &Path) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from("knowledge-primary"),
        resource_kind: ResourceKind::from(nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND),
        resource_id: ResourceId::from(knowledge_base_id.as_str()),
        owner_id: principal().principal_id,
        operations: BTreeSet::from(["read".to_owned(), "search".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([
            (
                KNOWLEDGE_ROOT_PARAMETER.to_owned(),
                root.to_string_lossy().into_owned(),
            ),
            (
                KNOWLEDGE_NAME_PARAMETER.to_owned(),
                "Release runbooks".to_owned(),
            ),
        ]),
    }
}

fn knowledge_invocation(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    binding: &TypedResourceBinding,
    capability_id: &str,
    input: serde_json::Value,
    request_id: &str,
) -> CapabilityInvocationRequest {
    let owner = principal();
    CapabilityInvocationRequest {
        principal: owner.clone(),
        session_owner: owner,
        agent_session_id: AgentSessionId::from("wave1-knowledge-session"),
        operation_id: OperationId::from(format!("wave1-knowledge-operation-{request_id}")),
        idempotency_key: IdempotencyKey::from(format!("wave1-knowledge-key-{request_id}")),
        correlation_id: CorrelationId::from(format!("wave1-knowledge-correlation-{request_id}")),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: active.generation,
        capability_id: CapabilityId::from(capability_id),
        action_id: nomifun_agent_domain_wave1::action_id(capability_id)
            .expect("Knowledge capability action"),
        resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
        state_scope_key: ScopeKey::from("session:wave1-knowledge-session"),
        input: StrictJsonValue(input),
    }
}

fn compile_wave1_snapshot_for_registry(
    registry: &KernelRegistry,
    capability_ids: &[&str],
    binding: TypedResourceBinding,
    fixture_name: &str,
) -> (
    Arc<CompiledSnapshot>,
    ActiveCapabilitySetSnapshot,
    TypedResourceBinding,
) {
    let owner = principal();
    let enabled_capabilities = capability_ids
        .iter()
        .copied()
        .map(|capability_id| CapabilitySelection {
            capability: CapabilityRef {
                id: capability_id.into(),
                version: VersionString::from(CONTRACT_VERSION),
            },
            action_allowlist: nomifun_agent_domain_wave1::action_id(capability_id)
                .into_iter()
                .collect(),
        })
        .collect();
    let materialized = registry.snapshot().expect("registry snapshot");
    let contribution_locks = capability_ids
        .iter()
        .map(|capability_id| {
            materialized
                .capability(&CapabilityId::from(*capability_id))
                .expect("selected Wave 1 capability is materialized")
                .contribution_lock
                .clone()
        })
        .collect();
    let payload = AgentPresetRevisionPayload {
        runtime_engine: None,
        schema_version: VersionString::from(CONTRACT_VERSION),
        model_route_refs: BTreeMap::new(),
        chat_route_records: BTreeMap::new(),
        enabled_capabilities,
        skill_bindings: Vec::new(),
        system_role_provider_overrides: BTreeMap::new(),
        persona: format!("Wave 1 {fixture_name} test"),
        instructions: format!("Exercise the Wave 1 {fixture_name} owner."),
        starter_prompts: Vec::new(),
    };
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from(format!("wave1-{fixture_name}")),
            revision: 1,
            revision_digest: DigestHex::from(""),
        },
        payload,
        contribution_locks,
        created_by: UserId::from(owner.principal_id.clone()),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().expect("revision digest");
    let snapshot = AgentPresetCompiler::compile(
        &materialized,
        &CompilerEnvironment {
            resolver_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("runtime"),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("target"),
            host_target: RuntimeTarget::from("test-target"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: format!("wave1-{fixture_name}-test"),
        },
        CompileRequest {
            plugin_product_capabilities: Vec::new(),
            revision,
            principal: owner.clone(),
            scene: format!("wave1-{fixture_name}-test"),
            surface: "desktop".to_owned(),
            audience: "test".to_owned(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from(format!("wave1-{fixture_name}-resolve")),
        },
    )
    .expect("compile Wave 1 capabilities")
    .with_target_resource_bindings(&owner, vec![binding.clone()])
    .expect("bind Wave 1 target resource");
    let active = SessionCapabilityState::new(&snapshot)
        .snapshot()
        .expect("fixed enabled capability set");
    assert_eq!(active.generation, 0);
    assert_eq!(active.resolved_snapshot_ref, *snapshot.snapshot_ref());
    assert_eq!(active.active, snapshot.content().capability_allowlist);
    assert_eq!(
        active.active,
        snapshot.content().enabled_capabilities.iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>()
    );
    for capability_id in capability_ids {
        assert!(active.active.contains(&CapabilityId::from(*capability_id)));
    }
    (Arc::new(snapshot), active, binding)
}

struct MemoryKernelFixture {
    registry: Arc<KernelRegistry>,
    persistence: Arc<InMemoryPluginStatePersistence>,
}

impl MemoryKernelFixture {
    fn new() -> Self {
        let persistence = Arc::new(InMemoryPluginStatePersistence::new());
        Self::with_persistence(persistence)
    }

    fn with_persistence(persistence: Arc<InMemoryPluginStatePersistence>) -> Self {
        let registry = Arc::new(
            KernelRegistry::new(
                MaterializationPolicy::stable(CONTRACT_VERSION),
                Arc::clone(&persistence) as Arc<dyn PluginStatePersistence>,
            )
            .expect("kernel registry"),
        );
        registry
            .replace_all(
                nomifun_agent_domain_wave1::registrations_with_host_port(Arc::new(
                    Wave1ApplicationHost::default(),
                ))
                .expect("Wave 1 registrations"),
            )
            .expect("publish Wave 1 registrations");
        Self {
            registry,
            persistence,
        }
    }

    fn compile_memory_snapshot(
        &self,
        capability_id: &str,
        resource_id: &str,
    ) -> (
        Arc<CompiledSnapshot>,
        ActiveCapabilitySetSnapshot,
        TypedResourceBinding,
    ) {
        compile_memory_snapshot_for_registry(&self.registry, capability_id, resource_id)
    }
}

fn compile_memory_snapshot_for_registry(
    registry: &KernelRegistry,
    capability_id: &str,
    resource_id: &str,
) -> (
    Arc<CompiledSnapshot>,
    ActiveCapabilitySetSnapshot,
    TypedResourceBinding,
) {
    let binding = TypedResourceBinding {
        binding_id: ResourceBindingId::from(format!("binding-{}", resource_id)),
        resource_kind: ResourceKind::from(if capability_id.starts_with("memory.project.") {
            nomifun_agent_domain_wave1::PROJECT_MEMORY_RESOURCE_KIND
        } else {
            nomifun_agent_domain_wave1::COMPANION_MEMORY_RESOURCE_KIND
        }),
        resource_id: ResourceId::from(resource_id),
        owner_id: principal().principal_id,
        operations: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    };
    let fixture_name = format!("memory-{}", capability_id.replace('.', "-"));
    compile_wave1_snapshot_for_registry(registry, &[capability_id], binding, &fixture_name)
}

fn memory_invocation(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    binding: &TypedResourceBinding,
    capability_id: &str,
    idempotency_key: &str,
    content: &str,
    operation_id: &str,
) -> CapabilityInvocationRequest {
    let owner = principal();
    CapabilityInvocationRequest {
        principal: owner.clone(),
        session_owner: owner,
        agent_session_id: AgentSessionId::from("wave1-memory-session"),
        operation_id: OperationId::from(operation_id),
        idempotency_key: IdempotencyKey::from(idempotency_key),
        correlation_id: CorrelationId::from(format!("correlation-{idempotency_key}")),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: active.generation,
        capability_id: CapabilityId::from(capability_id),
        action_id: nomifun_agent_domain_wave1::action_id(capability_id)
            .expect("memory capability action"),
        resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
        state_scope_key: ScopeKey::from("session:wave1-memory-session"),
        input: StrictJsonValue(serde_json::json!({
            "content": content
        })),
    }
}

fn companion_memory_invocation(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    binding: &TypedResourceBinding,
    capability_id: &str,
    idempotency_key: &str,
    input: serde_json::Value,
) -> CapabilityInvocationRequest {
    let owner = principal();
    CapabilityInvocationRequest {
        principal: owner.clone(),
        session_owner: owner,
        agent_session_id: AgentSessionId::from("0199a000-0000-7000-8000-000000000001"),
        operation_id: OperationId::from(format!("wave1-companion-memory-{idempotency_key}")),
        idempotency_key: IdempotencyKey::from(idempotency_key),
        correlation_id: CorrelationId::from(format!(
            "wave1-companion-memory-correlation-{idempotency_key}"
        )),
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: active.generation,
        capability_id: CapabilityId::from(capability_id),
        action_id: nomifun_agent_domain_wave1::action_id(capability_id)
            .expect("Companion memory capability action"),
        resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
        state_scope_key: ScopeKey::from("session:0199a000-0000-7000-8000-000000000001"),
        input: StrictJsonValue(input),
    }
}

fn malformed_memory_persistence() -> Arc<InMemoryPluginStatePersistence> {
    memory_persistence_with_state(
        serde_json::json!({
            "entries": "not-an-array"
        }),
        MEMORY_STATE_FORMAT_VERSION,
        "project-corrupt",
    )
}

fn memory_persistence_with_state(
    value: serde_json::Value,
    state_format_version: &str,
    resource_scope: &str,
) -> Arc<InMemoryPluginStatePersistence> {
    let identity = StateIdentity {
        package_id: nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
        mount_id: nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
        scope_key: ScopeKey::from(format!("resource:{resource_scope}")),
        state_key: StateKey::from(MEMORY_STATE_KEY),
    };
    let entry = PluginStateEntry {
        namespace: identity.namespace(),
        revision: 1,
        state_format_version: VersionString::from(state_format_version),
        writer_package_version: VersionString::from(CONTRACT_VERSION),
        value: StrictJsonValue(value),
    };
    let snapshot = PluginStateSnapshot::from_parts(
        BTreeMap::from([(identity.clone(), entry)]),
        BTreeMap::from([(identity, 1)]),
    )
    .expect("malformed fixture namespace");
    Arc::new(InMemoryPluginStatePersistence::reopen(snapshot))
}

#[test]
fn wave1_knowledge_binding_resolution_rechecks_host_authority() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("knowledge");
    std::fs::create_dir_all(&root).unwrap();
    let knowledge_base_id = KnowledgeBaseId::new();
    let binding = knowledge_binding(&knowledge_base_id, &root);
    let capability_id = CapabilityId::from(nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH);
    let action_id =
        nomifun_agent_domain_wave1::action_id(nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH)
            .unwrap();
    let principal_id = principal().principal_id;
    let resolve = |bindings: &[TypedResourceBinding]| {
        resolve_bound_knowledge_base_parts(
            &principal_id,
            &capability_id,
            &action_id,
            bindings,
            nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
            "search",
        )
    };
    let error_code =
        |bindings: &[TypedResourceBinding]| resolve(bindings).unwrap_err().code.as_ref().to_owned();

    let resolved = resolve(std::slice::from_ref(&binding)).expect("valid binding");
    assert_eq!(
        resolved.knowledge_base_id().as_str(),
        knowledge_base_id.as_str()
    );

    let mut wrong_owner = binding.clone();
    wrong_owner.owner_id = "different-owner".to_owned();
    assert_eq!(error_code(&[wrong_owner]), "RESOURCE_OWNER_MISMATCH");

    let mut missing_grant = binding.clone();
    missing_grant.operations.remove("search");
    assert_eq!(error_code(&[missing_grant]), "PRESET_RESOURCE_NOT_BOUND");

    let mut invalid_id = binding.clone();
    invalid_id.resource_id = ResourceId::from("not-a-uuidv7");
    assert_eq!(error_code(&[invalid_id]), "INVALID_PAYLOAD");

    let mut missing_root = binding.clone();
    missing_root
        .typed_parameters
        .remove(KNOWLEDGE_ROOT_PARAMETER);
    assert_eq!(error_code(&[missing_root]), "PRESET_RESOURCE_NOT_BOUND");

    let mut second_binding = binding.clone();
    second_binding.binding_id = ResourceBindingId::from("knowledge-secondary");
    assert_eq!(
        error_code(&[binding, second_binding]),
        "PRESET_RESOURCE_NOT_BOUND"
    );
}

#[tokio::test]
async fn wave1_knowledge_owner_searches_and_reads_real_bound_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("knowledge");
    std::fs::create_dir_all(root.join("release")).unwrap();
    let content = "# Release rollback\nRun the signed rollback plan.";
    std::fs::write(root.join("release").join("rollback.md"), content).unwrap();

    let fixture = KnowledgeKernelFixture::new();
    let knowledge_base_id = KnowledgeBaseId::new();
    let binding = knowledge_binding(&knowledge_base_id, &root);
    let (snapshot, active, binding) = fixture.compile_snapshot(binding);
    let search = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
            serde_json::json!({
                "query": "rollback",
                "limit": 5,
            }),
            "search",
        )
        .await
        .expect("bound Knowledge search");
    assert_eq!(search.0["total"], serde_json::json!(1));
    assert_eq!(
        search.0["hits"][0]["resource_id"],
        serde_json::json!(knowledge_base_id)
    );
    assert_eq!(
        search.0["hits"][0]["relative_path"],
        serde_json::json!("release/rollback.md")
    );
    assert!(
        !search
            .0
            .to_string()
            .contains(&root.to_string_lossy().to_string()),
        "Knowledge search must not expose an absolute host path"
    );
    let handle = search.0["hits"][0]["handle"]
        .as_str()
        .expect("search handle")
        .to_owned();

    let read = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            serde_json::json!({ "handle": handle }),
            "read",
        )
        .await
        .expect("bound Knowledge read");
    assert_eq!(read.0["content"], serde_json::json!(content));
    assert_eq!(
        read.0["relative_path"],
        serde_json::json!("release/rollback.md")
    );
    assert_eq!(
        read.0["content_sha256"],
        serde_json::json!(digest_bytes(content.as_bytes()))
    );
}

#[tokio::test]
async fn wave1_knowledge_owner_rejects_scope_escape_and_missing_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("knowledge");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("note.md"), "# In scope").unwrap();
    std::fs::write(directory.path().join("outside.md"), "# Outside").unwrap();

    let fixture = KnowledgeKernelFixture::new();
    let knowledge_base_id = KnowledgeBaseId::new();
    let binding = knowledge_binding(&knowledge_base_id, &root);
    let (snapshot, active, binding) = fixture.compile_snapshot(binding);

    let wrong_resource = nomifun_knowledge::encode_doc_handle(&KnowledgeBaseId::new(), "note.md");
    let wrong_resource_error = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            serde_json::json!({ "handle": wrong_resource }),
            "wrong-resource",
        )
        .await
        .expect_err("a handle cannot widen the bound resource scope");
    assert!(
        wrong_resource_error
            .to_string()
            .contains("PRESET_RESOURCE_NOT_BOUND"),
        "unexpected wrong-resource error: {wrong_resource_error}"
    );

    let traversal = nomifun_knowledge::encode_doc_handle(&knowledge_base_id, "../outside.md");
    let traversal_error = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            serde_json::json!({ "handle": traversal }),
            "traversal",
        )
        .await
        .expect_err("a handle cannot traverse outside its bound root");
    assert!(
        traversal_error.to_string().contains("INVALID_PAYLOAD"),
        "unexpected traversal error: {traversal_error}"
    );

    let missing = nomifun_knowledge::encode_doc_handle(&knowledge_base_id, "missing.md");
    let missing_error = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            serde_json::json!({ "handle": missing }),
            "missing-file",
        )
        .await
        .expect_err("a missing file must not produce synthetic content");
    assert!(
        missing_error.to_string().contains("RESOURCE_NOT_FOUND"),
        "unexpected missing-file error: {missing_error}"
    );
}

#[tokio::test]
async fn wave1_knowledge_owner_fails_closed_for_missing_bound_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("missing-knowledge-root");
    let fixture = KnowledgeKernelFixture::new();
    let knowledge_base_id = KnowledgeBaseId::new();
    let binding = knowledge_binding(&knowledge_base_id, &root);
    let (snapshot, active, binding) = fixture.compile_snapshot(binding);

    let error = fixture
        .invoke(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
            serde_json::json!({ "query": "anything" }),
            "missing-root",
        )
        .await
        .expect_err("a missing root must fail instead of returning no hits");
    assert!(
        error.to_string().contains("CAPABILITY_UNAVAILABLE"),
        "unexpected missing-root error: {error}"
    );
    assert!(
        !error
            .to_string()
            .contains(&root.to_string_lossy().to_string()),
        "Knowledge errors must not expose an absolute host path"
    );
}

#[tokio::test]
async fn wave1_companion_memory_kernel_path_uses_the_persistent_domain_owner() {
    let directory = tempfile::tempdir().expect("Companion data root");
    let database = nomifun_db::init_database_memory()
        .await
        .expect("receipt database");
    let companion_service = companion_service(directory.path()).await;
    let profile = companion_service
        .create_companion("Kernel Companion", "ink")
        .await
        .expect("create Companion");
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable(CONTRACT_VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .expect("Kernel registry");
    registry
        .replace_all(
            nomifun_agent_domain_wave1::registrations_with_host_port(Arc::new(
                Wave1ApplicationHost::with_companion_memory(
                    Arc::clone(&companion_service),
                    database.pool().clone(),
                ),
            ))
            .expect("Wave 1 registrations"),
        )
        .expect("publish Wave 1 registrations");

    let invoke = |capability_id: &'static str,
                  input: serde_json::Value,
                  request_id: &'static str| {
        let (snapshot, active, binding) =
            compile_memory_snapshot_for_registry(&registry, capability_id, &profile.companion_id);
        let registry = &registry;
        async move {
            registry
                .invoke(
                    &snapshot,
                    &active,
                    companion_memory_invocation(
                        &snapshot,
                        &active,
                        &binding,
                        capability_id,
                        request_id,
                        input,
                    ),
                )
                .await
        }
    };
    let first = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
        serde_json::json!({
            "kind": "preference",
            "content": "Prefer concrete verification evidence.",
            "tags": ["verification"]
        }),
        "write-first",
    )
    .await
    .expect("first durable Companion memory");
    let first_replay = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
        serde_json::json!({
            "kind": "preference",
            "content": "Prefer concrete verification evidence.",
            "tags": ["verification"]
        }),
        "write-first",
    )
    .await
    .expect("write replay returns its durable receipt");
    assert_eq!(
        first_replay, first,
        "a write replay must not reinforce the memory a second time"
    );
    let second = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
        serde_json::json!({
            "kind": "preference",
            "content": "Preserve unrelated changes.",
            "tags": ["git"]
        }),
        "write-second",
    )
    .await
    .expect("second durable Companion memory");
    let first_id = first.0["memory_id"].as_str().unwrap().to_owned();
    let second_id = second.0["memory_id"].as_str().unwrap().to_owned();

    let merge_input = serde_json::json!({
        "memory_ids": [first_id, second_id],
        "merged_content": "Preserve unrelated changes and cite concrete verification evidence.",
        "kind": "preference"
    });
    let merged = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_MERGE,
        merge_input.clone(),
        "merge",
    )
    .await
    .expect("atomic Companion memory merge");
    assert_eq!(merged.0["source"], serde_json::json!("merge"));
    let merged_replay = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_MERGE,
        merge_input,
        "merge",
    )
    .await
    .expect("merge replay succeeds after its source memories were archived");
    assert_eq!(merged_replay, merged);
    let merged_id = merged.0["memory_id"].as_str().unwrap().to_owned();

    let evolve_input = serde_json::json!({
        "memory_id": merged_id,
        "content": "Preserve unrelated changes, cite checks, and name remaining blockers."
    });
    let evolved = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_EVOLVE,
        evolve_input.clone(),
        "evolve",
    )
    .await
    .expect("in-place Companion memory evolution");
    assert_eq!(evolved.0["memory_id"], merged.0["memory_id"]);
    assert!(
        evolved.0["content"]
            .as_str()
            .unwrap()
            .contains("remaining blockers")
    );
    let evolved_replay = invoke(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_EVOLVE,
        evolve_input,
        "evolve",
    )
    .await
    .expect("evolve replay returns the original durable result");
    assert_eq!(evolved_replay, evolved);

    let (snapshot, active, binding) = compile_memory_snapshot_for_registry(
        &registry,
        nomifun_agent_domain_wave1::MEMORY_COMPANION_RECALL,
        &profile.companion_id,
    );
    let recalled = registry
        .contribute_context(
            &snapshot,
            &active,
            CapabilityAccessRequest {
                principal: principal(),
                session_owner: principal(),
                agent_session_id: AgentSessionId::from("0199a000-0000-7000-8000-000000000001"),
                operation_id: OperationId::from("wave1-companion-memory-recall"),
                correlation_id: CorrelationId::from("wave1-companion-memory-recall-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from(
                    nomifun_agent_domain_wave1::MEMORY_COMPANION_RECALL,
                ),
                resource_binding_ids: BTreeSet::from([binding.binding_id]),
                state_scope_key: ScopeKey::from("session:0199a000-0000-7000-8000-000000000001"),
            },
        )
        .await
        .expect("persistent Companion memory recall")
        .value
        .expect("Companion context value")
        .0;
    assert_eq!(
        recalled["companion_id"],
        serde_json::json!(profile.companion_id)
    );
    assert_eq!(recalled["memories"].as_array().unwrap().len(), 1);
    assert_eq!(recalled["memories"][0]["memory_id"], evolved.0["memory_id"]);
    assert_eq!(recalled["memories"][0]["content"], evolved.0["content"]);
    database.close().await;
}

#[tokio::test]
async fn wave1_memory_owner_persists_and_replays_by_request_identity() {
    let fixture = MemoryKernelFixture::new();
    let (snapshot, active, binding) = fixture.compile_memory_snapshot(
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "project-replay",
    );
    let request = memory_invocation(
        &snapshot,
        &active,
        &binding,
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "memory-replay-1",
        "remember this",
        "memory-operation-1",
    );
    let first = fixture
        .registry
        .invoke(&snapshot, &active, request.clone())
        .await
        .expect("first memory mutation");
    assert_eq!(first.0["persisted"], serde_json::json!(true));
    assert_eq!(first.0["revision"], serde_json::json!(1));
    assert_eq!(first.0["entry_count"], serde_json::json!(1));

    let state = fixture.persistence.snapshot().expect("state snapshot");
    let stored = state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-replay"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("project memory state");
    assert_eq!(stored.revision, 1);
    assert_eq!(stored.value.0["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        stored.value.0["entries"][0]["request"]["content"],
        serde_json::json!("remember this")
    );

    let mut replay_request = request.clone();
    replay_request.operation_id = OperationId::from("memory-operation-retry");
    replay_request.correlation_id = CorrelationId::from("memory-correlation-retry");
    let replay = fixture
        .registry
        .invoke(&snapshot, &active, replay_request)
        .await
        .expect("idempotent replay");
    assert_eq!(replay, first);
    let replayed_state = fixture.persistence.snapshot().expect("state snapshot");
    let replayed = replayed_state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-replay"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("project memory state after replay");
    assert_eq!(replayed.revision, 1);
    assert_eq!(replayed.value.0["entries"].as_array().unwrap().len(), 1);

    let conflicting_request = memory_invocation(
        &snapshot,
        &active,
        &binding,
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "memory-replay-1",
        "different content",
        "memory-operation-conflict",
    );
    let conflict = fixture
        .registry
        .invoke(&snapshot, &active, conflicting_request)
        .await
        .expect_err("different input must conflict");
    assert!(
        conflict.to_string().contains("IDEMPOTENCY_CONFLICT"),
        "unexpected conflict: {conflict}"
    );
    let unchanged = fixture.persistence.snapshot().expect("state snapshot");
    let unchanged_entry = unchanged
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-replay"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("project memory state after conflict");
    assert_eq!(unchanged_entry.revision, 1);
    assert_eq!(
        unchanged_entry.value.0["entries"].as_array().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn wave1_project_memory_owner_isolates_resources() {
    let fixture = MemoryKernelFixture::new();
    let (project_a, active_a, binding_a) = fixture
        .compile_memory_snapshot(nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE, "memory-a");
    let (project_b, active_b, binding_b) = fixture
        .compile_memory_snapshot(nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE, "memory-b");
    for (snapshot, active, binding, capability_id, content) in [
        (
            project_a,
            active_a,
            binding_a,
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "project a",
        ),
        (
            project_b,
            active_b,
            binding_b,
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "project b",
        ),
    ] {
        fixture
            .registry
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    capability_id,
                    "shared-idempotency-key",
                    content,
                    content,
                ),
            )
            .await
            .expect("isolated memory mutation");
    }

    let state = fixture.persistence.snapshot().expect("state snapshot");
    for (package_id, mount_id, scope, expected_content) in [
        (
            nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID,
            nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID,
            "resource:memory-a",
            "project a",
        ),
        (
            nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID,
            nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID,
            "resource:memory-b",
            "project b",
        ),
    ] {
        let entry = state
            .entry(
                &package_id.into(),
                &mount_id.into(),
                &ScopeKey::from(scope),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("isolated state entry");
        assert_eq!(entry.value.0["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            entry.value.0["entries"][0]["request"]["content"],
            serde_json::json!(expected_content)
        );
    }
}

#[tokio::test]
async fn wave1_project_memory_owner_dispatches_both_mutation_variants() {
    let fixture = MemoryKernelFixture::new();
    for (index, (capability_id, resource_id, expected_operation)) in [
        (
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "variant-project",
            "project.write",
        ),
        (
            nomifun_agent_domain_wave1::MEMORY_PROJECT_DISTILL,
            "variant-project",
            "project.distill",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (snapshot, active, binding) =
            fixture.compile_memory_snapshot(capability_id, resource_id);
        let output = fixture
            .registry
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    capability_id,
                    &format!("variant-key-{index}"),
                    &format!("variant-{index}"),
                    &format!("variant-operation-{index}"),
                ),
            )
            .await
            .expect("memory mutation variant");
        assert_eq!(output.0["operation"], serde_json::json!(expected_operation));
    }
}

#[tokio::test]
async fn wave1_memory_owner_survives_kernel_restart_and_concurrent_cas() {
    let fixture = MemoryKernelFixture::new();
    let (snapshot, active, binding) = fixture.compile_memory_snapshot(
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "project-restart",
    );
    let first_request = memory_invocation(
        &snapshot,
        &active,
        &binding,
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "restart-key",
        "before restart",
        "restart-operation",
    );
    let first = fixture
        .registry
        .invoke(&snapshot, &active, first_request.clone())
        .await
        .expect("pre-restart mutation");
    let persisted_snapshot = fixture.persistence.snapshot().expect("persisted state");

    let restarted_persistence =
        Arc::new(InMemoryPluginStatePersistence::reopen(persisted_snapshot));
    let restarted = MemoryKernelFixture::with_persistence(restarted_persistence.clone());
    let (restarted_snapshot, restarted_active, restarted_binding) = restarted
        .compile_memory_snapshot(
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "project-restart",
        );
    let replay = restarted
        .registry
        .invoke(
            &restarted_snapshot,
            &restarted_active,
            memory_invocation(
                &restarted_snapshot,
                &restarted_active,
                &restarted_binding,
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "restart-key",
                "before restart",
                "restart-operation-retry",
            ),
        )
        .await
        .expect("post-restart replay");
    assert_eq!(replay, first);

    let task_count = 12;
    let mut tasks = Vec::with_capacity(task_count);
    for index in 0..task_count {
        let registry = Arc::clone(&restarted.registry);
        let snapshot = Arc::clone(&restarted_snapshot);
        let active = restarted_active.clone();
        let binding = restarted_binding.clone();
        tasks.push(tokio::spawn(async move {
            registry
                .invoke(
                    &snapshot,
                    &active,
                    memory_invocation(
                        &snapshot,
                        &active,
                        &binding,
                        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                        &format!("concurrent-key-{index}"),
                        &format!("concurrent-{index}"),
                        &format!("concurrent-operation-{index}"),
                    ),
                )
                .await
        }));
    }
    for task in tasks {
        task.await
            .expect("concurrent task")
            .expect("concurrent CAS mutation");
    }
    let state = restarted_persistence.snapshot().expect("restarted state");
    let entry = state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-restart"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("restarted project memory state");
    assert_eq!(
        entry.value.0["entries"].as_array().unwrap().len(),
        task_count + 1
    );
    assert_eq!(entry.revision, task_count as u64 + 1);
}

#[tokio::test]
async fn wave1_memory_owner_enforces_bounded_state_without_partial_append() {
    let fixture = MemoryKernelFixture::new();
    let (snapshot, active, binding) = fixture.compile_memory_snapshot(
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "project-capacity",
    );
    let content = "x".repeat(1_024);
    let mut successful = 0usize;
    let mut terminal_error = None;
    for index in 0..MAX_MEMORY_ENTRIES {
        let result = fixture
            .registry
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    &format!("capacity-key-{index}"),
                    &content,
                    &format!("capacity-operation-{index}"),
                ),
            )
            .await;
        match result {
            Ok(_) => successful += 1,
            Err(error) => {
                terminal_error = Some(error);
                break;
            }
        }
    }
    let terminal_error = terminal_error.expect("bounded state must eventually reject");
    assert!(
        terminal_error
            .to_string()
            .contains("CAPABILITY_UNAVAILABLE"),
        "unexpected capacity error: {terminal_error}"
    );
    assert!(successful > 0 && successful < MAX_MEMORY_ENTRIES);

    let state = fixture.persistence.snapshot().expect("state snapshot");
    let entry = state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-capacity"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("capacity state");
    assert_eq!(entry.revision, successful as u64);
    assert_eq!(
        entry.value.0["entries"].as_array().unwrap().len(),
        successful
    );
}

#[tokio::test]
async fn wave1_memory_owner_rejects_corrupt_plugin_state_without_overwrite() {
    let persistence = malformed_memory_persistence();
    let fixture = MemoryKernelFixture::with_persistence(Arc::clone(&persistence));
    let (snapshot, active, binding) = fixture.compile_memory_snapshot(
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "project-corrupt",
    );
    let error = fixture
        .registry
        .invoke(
            &snapshot,
            &active,
            memory_invocation(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "corrupt-repair-attempt",
                "must not overwrite",
                "corrupt-operation",
            ),
        )
        .await
        .expect_err("corrupt state must fail closed");
    assert!(
        error.to_string().contains("CAPABILITY_UNAVAILABLE"),
        "unexpected corrupt-state error: {error}"
    );
    let state = persistence.snapshot().expect("state snapshot");
    let entry = state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-corrupt"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("corrupt state remains");
    assert_eq!(entry.revision, 1);
    assert_eq!(entry.value.0["entries"], serde_json::json!("not-an-array"));
}

#[tokio::test]
async fn wave1_memory_owner_rejects_unsupported_state_format() {
    let persistence = memory_persistence_with_state(
        serde_json::json!({"entries": []}),
        "2.0.0",
        "project-format",
    );
    let fixture = MemoryKernelFixture::with_persistence(Arc::clone(&persistence));
    let (snapshot, active, binding) = fixture.compile_memory_snapshot(
        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
        "project-format",
    );
    let error = fixture
        .registry
        .invoke(
            &snapshot,
            &active,
            memory_invocation(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "format-key",
                "must not migrate implicitly",
                "format-operation",
            ),
        )
        .await
        .expect_err("unsupported state format must fail closed");
    assert!(
        error.to_string().contains("CAPABILITY_UNAVAILABLE"),
        "unexpected state-format error: {error}"
    );
    let state = persistence.snapshot().expect("state snapshot");
    let entry = state
        .entry(
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            &ScopeKey::from("resource:project-format"),
            &StateKey::from(MEMORY_STATE_KEY),
        )
        .expect("format state remains");
    assert_eq!(entry.state_format_version.as_ref(), "2.0.0");
    assert_eq!(entry.value.0["entries"], serde_json::json!([]));
}

#[test]
fn wave1_application_errors_keep_typed_owner_codes() {
    let invalid =
        wave1_application_error(nomifun_common::AppError::BadRequest("bad URL".to_owned()));
    assert_eq!(invalid.code.as_ref(), "INVALID_PAYLOAD");

    let unavailable = wave1_application_error(nomifun_common::AppError::Timeout(
        "network timeout".to_owned(),
    ));
    assert_eq!(unavailable.code.as_ref(), "CAPABILITY_UNAVAILABLE");

    let hidden_path = PathBuf::from(r"C:\Users\owner\private-knowledge");
    let knowledge = wave1_bound_knowledge_error(nomifun_common::AppError::Internal(format!(
        "failed to inspect {}",
        hidden_path.display()
    )));
    assert_eq!(knowledge.code.as_ref(), "CAPABILITY_UNAVAILABLE");
    assert!(!knowledge.message.contains("private-knowledge"));
}
