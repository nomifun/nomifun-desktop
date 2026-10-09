use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentPresetId, AgentPresetRevision,
    AgentPresetRevisionPayload, AgentSessionId, ArtifactEnvelope,
    CancellationDescriptor, CapabilityActionDescriptor,
    CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind, CapabilityManifest, CapabilityRef,
    CapabilitySelection, CanonicalSchemaRef,
    DeclaredServiceViewDescriptor, DigestHex, EffectClass, HostPortId,
    HostPortBindingDescriptor, HostPortRef, InProcessEntrypointMetadata,
    LocalizedMetadata,
    ManagedTaskRegistrationDescriptor, OperationId, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint,
    PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    AgentModuleId, PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor,
    PluginRegistrarDescriptor, PluginRegistrarOperation,
    PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateHandleDescriptor, PluginStateMethod, PresetRevisionRef,
    PrincipalRef,
    RuntimeProfileKind, RuntimeTarget, ScopeKey,
    StrictJsonValue, ToolPresentationKind, UserId,
    ValidatedPluginConfig, VersionString, capability_surface_declarations,
    digest_payload,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler, CapabilityContextContributionFactory,
    CapabilityContextContributionRequest, CapabilityHandler,
    CapabilityInvocationContext, CompileRequest, CompilerEnvironment,
    ContextContributionResult, InMemoryPluginStatePersistence, KernelError,
    KernelRegistry, MaterializationPolicy, PluginRegistration,
    PluginStatePersistence,
};
use nomifun_ai_agent::{
    NomiPluginToolError,
};
use serde_json::{Value, json};

const VERSION: &str = "1.0.0";
const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000002";
const PACKAGE: &str = "example.dynamic-plugin";
const MOUNT: &str = "0190f5fe-7c00-7a00-8000-000000000003";
const AGENT_TOOL: &str = "example.dynamic.echo";
const AGENT_ACTION: &str = "example.dynamic.echo.invoke";
const HIDDEN_ACTION: &str = "example.dynamic.echo.internal";
const UI_ONLY_TOOL: &str = "example.dynamic.ui-only";
const UI_ONLY_ACTION: &str = "example.dynamic.ui-only.invoke";
const CONTEXT_CAPABILITY: &str = "example.dynamic.context";

#[path = "plugin_tool_consumer/dependencies.rs"]
mod dependency_consumer;

#[derive(Clone, Debug, PartialEq, Eq)]
struct InvocationEvidence {
    operation_id: String,
    idempotency_key: String,
    correlation_id: String,
    agent_session_id: String,
    capability_id: String,
    action_id: String,
    target_artifact_digest: String,
}

struct CapturingHandler {
    prefix: &'static str,
    calls: Arc<AtomicUsize>,
    evidence: Arc<Mutex<Vec<InvocationEvidence>>>,
}

#[async_trait]
impl CapabilityHandler for CapturingHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.evidence.lock().unwrap().push(InvocationEvidence {
            operation_id: context.operation_id.as_ref().to_owned(),
            idempotency_key: context.idempotency_key.as_ref().to_owned(),
            correlation_id: context.correlation_id.as_ref().to_owned(),
            agent_session_id: context.agent_session_id.as_ref().to_owned(),
            capability_id: context.capability_id.as_ref().to_owned(),
            action_id: context.action_id.as_ref().to_owned(),
            target_artifact_digest: context
                .resolved_capability
                .target_artifact_digest
                .as_ref()
                .to_owned(),
        });
        Ok(StrictJsonValue(json!({
            "echo": format!(
                "{}{}",
                self.prefix,
                input.0["message"].as_str().unwrap_or_default()
            )
        })))
    }
}

struct EmptyContextFactory;

#[async_trait]
impl CapabilityContextContributionFactory for EmptyContextFactory {
    async fn contribute(
        &self,
        _request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        Ok(ContextContributionResult {
            value: Some(StrictJsonValue(json!({
                "source": "bundled-context-fixture",
                "role": "assistant"
            }))),
        })
    }
}

fn display(name: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: format!("{name} description"),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn schema_ref(subject: &str, schema: &Value) -> CanonicalSchemaRef {
    CanonicalSchemaRef::from(format!(
        "schema://{subject}@1#{}",
        digest_payload(schema).unwrap().as_ref()
    ))
}

fn action(
    action_id: &str,
    input_schema: &Value,
    presentation: ToolPresentationKind,
) -> CapabilityActionDescriptor {
    let output_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "echo": {"type": "string"}
        },
        "required": ["echo"]
    });
    CapabilityActionDescriptor {
        action_id: ActionId::from(action_id),
        input_schema: schema_ref(&format!("{action_id}/input"), input_schema),
        output_schema: schema_ref(
            &format!("{action_id}/output"),
            &output_schema,
        ),
        effect_class: EffectClass::Pure,
        presentation,
    }
}

fn tool_capability(
    package: &PackageRef,
    id: &str,
    consumers: Vec<CapabilityConsumer>,
    actions: Vec<CapabilityActionDescriptor>,
) -> CapabilityManifest {
    CapabilityManifest {
        id: CapabilityId::from(id),
        contribution_id: format!("capability:{id}").into(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display(id),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            consumers,
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({
            "type": "object",
            "additionalProperties": false
        })),
        contributions: CapabilityContributions {
            actions,
            ..Default::default()
        },
    }
}

fn context_capability(package: &PackageRef) -> CapabilityManifest {
    let schema = json!({
        "type": "object",
        "additionalProperties": false
    });
    CapabilityManifest {
        id: CapabilityId::from(CONTEXT_CAPABILITY),
        contribution_id: format!("capability:{CONTEXT_CAPABILITY}").into(),
        kind: CapabilityKind::ContextContributor,
        package: package.clone(),
        display: display(CONTEXT_CAPABILITY),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({
            "type": "object",
            "additionalProperties": false
        })),
        contributions: CapabilityContributions {
            context_schema_refs: vec![schema_ref(
                "example.dynamic.context/value",
                &schema,
            )],
            ..Default::default()
        },
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(VERSION),
    }
}

fn registration(
    artifact_byte: char,
    prefix: &'static str,
    calls: Arc<AtomicUsize>,
    evidence: Arc<Mutex<Vec<InvocationEvidence>>>,
) -> PluginRegistration {
    registration_with_source(
        artifact_byte,
        prefix,
        calls,
        evidence,
        PluginSourceKind::ManagedLocal,
        false,
    )
}

fn registration_with_source(
    artifact_byte: char,
    prefix: &'static str,
    calls: Arc<AtomicUsize>,
    evidence: Arc<Mutex<Vec<InvocationEvidence>>>,
    source_kind: PluginSourceKind,
    declare_capability_host_port: bool,
) -> PluginRegistration {
    registration_with_context_factory(
        artifact_byte, prefix, calls, evidence, source_kind,
        declare_capability_host_port, Arc::new(EmptyContextFactory),
    )
}

fn registration_with_context_factory(
    artifact_byte: char,
    prefix: &'static str,
    calls: Arc<AtomicUsize>,
    evidence: Arc<Mutex<Vec<InvocationEvidence>>>,
    source_kind: PluginSourceKind,
    declare_capability_host_port: bool,
    context_factory: Arc<dyn CapabilityContextContributionFactory>,
) -> PluginRegistration {
    let package = PackageRef {
        id: PackageId::from(PACKAGE),
        version: VersionString::from(VERSION),
    };
    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {
                "type": "string",
                "maxLength": 256
            }
        },
        "required": ["message"]
    });
    let agent_action = action(
        AGENT_ACTION,
        &input_schema,
        ToolPresentationKind::FunctionTool,
    );
    let hidden_action = action(
        HIDDEN_ACTION,
        &input_schema,
        ToolPresentationKind::Hidden,
    );
    let ui_action = action(
        UI_ONLY_ACTION,
        &input_schema,
        ToolPresentationKind::FunctionTool,
    );
    let action_host = host_port("host.wave.fixture.invoke");
    let mut agent_capability = tool_capability(
            &package,
            AGENT_TOOL,
            vec![CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
            vec![agent_action.clone(), hidden_action],
        );
    if declare_capability_host_port {
        agent_capability.contributions.host_ports = vec![action_host.clone()];
    }
    let capabilities = vec![
        agent_capability,
        tool_capability(
            &package,
            UI_ONLY_TOOL,
            vec![CapabilityConsumer::Ui],
            vec![ui_action],
        ),
        context_capability(&package),
    ];
    let config_schema = StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false
    }));
    let manifest = PackageManifest {
        schema_version: VersionString::from(VERSION),
        host_contract_version: VersionString::from(VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(PACKAGE),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "test".to_owned(),
            entrypoint_id: "example.dynamic-plugin.entry".to_owned(),
            contract_version: VersionString::from(VERSION),
        }
        .into(),
        contributions: PackageContributions {
            capabilities,
            ..Default::default()
        },
    };
    let (source_identity, source_digest) = match source_kind {
        PluginSourceKind::ManagedLocal => (
            MOUNT.to_owned(),
            Some(DigestHex::from(artifact_byte.to_string().repeat(64))),
        ),
        PluginSourceKind::Bundled | PluginSourceKind::TestFixture => {
            (PACKAGE.to_owned(), None)
        }
    };
    let source = PluginSourceMetadata {
        source_kind,
        source_identity,
        source_digest,
    };
    let mount_id = AgentModuleId::from(MOUNT);
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount_id.clone(),
    };
    let cancel = host_port("host.plugin.cancel");
    let tasks = host_port("host.plugin.tasks");
    let action_host_binding = declare_capability_host_port.then(|| {
        let schema = json!({
            "type": "object",
            "additionalProperties": true
        });
        HostPortBindingDescriptor {
            port: action_host.clone(),
            request_schema: schema_ref("host.wave.fixture/request", &schema),
            response_schema: schema_ref(
                "host.wave.fixture/response",
                &schema,
            ),
        }
    });
    let mut declared_host_ports =
        BTreeSet::from([cancel.id.clone(), tasks.id.clone()]);
    if declare_capability_host_port {
        declared_host_ports.insert(action_host.id.clone());
    }
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).unwrap(),
        mount_id: mount_id.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::ContributeCapability,
                PluginRegistrarOperation::BindHostPort,
            ]),
            declared_capability_ids: BTreeSet::from([
                CapabilityId::from(AGENT_TOOL),
                CapabilityId::from(UI_ONLY_TOOL),
                CapabilityId::from(CONTEXT_CAPABILITY),
            ]),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).unwrap(),
                config_revision: 1,
                value: StrictJsonValue(json!({})),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id,
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: action_host_binding.into_iter().collect(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port: cancel,
                scope_key: ScopeKey::from(format!("mount:{MOUNT}")),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: tasks,
                scope_key: ScopeKey::from(format!("mount:{MOUNT}")),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    registration
        .add_capability_handler(
            CapabilityId::from(AGENT_TOOL),
            Arc::new(CapturingHandler {
                prefix,
                calls: Arc::clone(&calls),
                evidence: Arc::clone(&evidence),
            }),
        )
        .unwrap();
    registration
        .add_capability_handler(
            CapabilityId::from(UI_ONLY_TOOL),
            Arc::new(CapturingHandler {
                prefix: "ui:",
                calls,
                evidence,
            }),
        )
        .unwrap();
    registration
        .add_capability_context_factory(
            CapabilityId::from(CONTEXT_CAPABILITY),
            context_factory,
        )
        .unwrap();

    registration
}

fn policy() -> MaterializationPolicy {
    policy_for(PluginSourceKind::ManagedLocal)
}

fn policy_for(source_kind: PluginSourceKind) -> MaterializationPolicy {
    MaterializationPolicy {
        host_contract_version: VersionString::from(VERSION),
        available_runtime_features: BTreeSet::new(),
        allowed_sources: BTreeSet::from([source_kind]),
    }
}

fn owner() -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: OWNER.to_owned(),
    }
}

fn selection(id: &str, actions: &[&str]) -> CapabilitySelection {
    CapabilitySelection {
        capability: CapabilityRef {
            id: CapabilityId::from(id),
        },
        action_allowlist: actions
            .iter()
            .map(|action| ActionId::from(*action))
            .collect(),
    }
}

fn revision_with_capabilities(
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
) -> AgentPresetRevision {
    let tool = selection(AGENT_TOOL, &[AGENT_ACTION, HIDDEN_ACTION]);
    let context = selection(CONTEXT_CAPABILITY, &[]);
    let enabled_capabilities = vec![tool, context];
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from(
                "0190f5fe-7c00-7a00-8000-000000000004",
            ),
            revision: 1,
            revision_digest: DigestHex::from(""),
        },
        payload: AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from(VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities,
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        },
        contribution_locks: [AGENT_TOOL, CONTEXT_CAPABILITY]
            .into_iter()
            .map(|id| {
                materialized
                    .capability(&CapabilityId::from(id))
                    .unwrap()
                    .contribution_lock
                    .clone()
            })
            .collect(),
        created_by: UserId::from(OWNER),
        created_at_ms: 1,
        reason: None,
    };
    revision.contribution_locks.sort();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision
}

fn compile(
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
) -> nomifun_agent_kernel::CompiledSnapshot {
    compile_revision(materialized, revision_with_capabilities(materialized))
}

fn compile_revision(
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
    revision: AgentPresetRevision,
) -> nomifun_agent_kernel::CompiledSnapshot {
    try_compile_revision(materialized, revision).unwrap()
}

fn try_compile_revision(
    materialized: &nomifun_agent_kernel::MaterializedRegistry,
    revision: AgentPresetRevision,
) -> Result<nomifun_agent_kernel::CompiledSnapshot, KernelError> {
    AgentPresetCompiler::compile(
        materialized,
        &CompilerEnvironment {
            resolver_version: VersionString::from(VERSION),
            required_runtime_protocol_version: VersionString::from(VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from(
                "1".repeat(64),
            ),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from(
                "2".repeat(64),
            ),
            target_contribution_manifest_digest: materialized
                .registry_digest
                .clone(),
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: "plugin-tool-test".to_owned(),
        },
        CompileRequest {
            revision,
            principal: owner(),
            scene: "chat".to_owned(),
            surface: "desktop".to_owned(),
            audience: "owner".to_owned(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("plugin-tool-test-resolve"),
        },
    )
}

async fn initial_context(
    kernel: &Arc<KernelRegistry>,
    compiled: &nomifun_agent_kernel::CompiledSnapshot,
    admission: Option<&nomifun_ai_agent::NomiPlatformBuiltinContextAdmission>,
) -> Result<(Vec<nomifun_ai_agent::NomiInitialContextContribution>, Vec<CapabilityId>), NomiPluginToolError> {
    let registry = kernel.snapshot()?;
    let active = nomifun_agent_kernel::SessionCapabilityState::new(compiled).snapshot()?;
    nomifun_ai_agent::assemble_initial_capability_context(
        kernel, compiled, &active, &registry, &owner(), &AgentSessionId::from(SESSION),
        &ScopeKey::from(format!("session:{SESSION}")), admission,
    ).await
}

struct CapturingContextFactory {
    calls: Arc<AtomicUsize>,
    value: Option<StrictJsonValue>,
    fail: bool,
}

#[async_trait]
impl CapabilityContextContributionFactory for CapturingContextFactory {
    async fn contribute(
        &self,
        request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(request.schema_ref.as_ref().contains("example.dynamic.context/value"));
        if self.fail {
            return Err(KernelError::CapabilityExecution {
                reason: "context fixture unavailable".into(),
            });
        }
        Ok(ContextContributionResult { value: self.value.clone() })
    }
}

fn managed_context_fixture(
    artifact_byte: char,
    context_calls: Arc<AtomicUsize>,
    value: Option<StrictJsonValue>,
    fail: bool,
) -> PluginRegistration {
    registration_with_context_factory(
        artifact_byte, "tool:", Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())), PluginSourceKind::ManagedLocal,
        false, Arc::new(CapturingContextFactory { calls: context_calls, value, fail }),
    )
}

#[tokio::test]
async fn managed_context_reaches_prompt_through_the_canonical_context_consumer() {
    let calls = Arc::new(AtomicUsize::new(0));
    let value = StrictJsonValue(json!({"instructions": "Use the selected plugin context"}));
    let registration = managed_context_fixture('a', calls.clone(), Some(value.clone()), false);
    let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let compiled = compile(&materialized);
    let (contributions, turn_ids) = initial_context(&kernel, &compiled, None).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0].value(), &value);
    assert!(turn_ids.is_empty());
    let prompt = nomifun_ai_agent::render_initial_capability_context_section(&contributions).unwrap().unwrap();
    assert!(prompt.contains("Use the selected plugin context"));
    assert!(prompt.contains(CONTEXT_CAPABILITY));
}

#[tokio::test]
async fn unselected_managed_context_is_not_invoked() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registration = managed_context_fixture('a', calls.clone(), None, true);
    let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let mut revision = revision_with_capabilities(&materialized);
    revision.payload.enabled_capabilities.retain(|selection| selection.capability.id.as_ref() != CONTEXT_CAPABILITY);
    let context_lock = &materialized.capability(&CapabilityId::from(CONTEXT_CAPABILITY)).unwrap().contribution_lock;
    revision.contribution_locks.retain(|lock| lock != context_lock);
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let compiled = compile_revision(&materialized, revision);
    let (contributions, turn_ids) = initial_context(&kernel, &compiled, None).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(contributions.is_empty());
    assert!(turn_ids.is_empty());
    assert_eq!(nomifun_ai_agent::render_initial_capability_context_section(&contributions).unwrap(), None);
}

#[tokio::test]
async fn managed_context_drift_or_withdrawal_fails_before_replacement_dispatch() {
    for withdraw in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let original = managed_context_fixture('a', calls.clone(), None, false);
        let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
        let materialized = kernel.replace_all(vec![original]).unwrap();
        let compiled = compile(&materialized);
        let replacement = managed_context_fixture('b', calls.clone(), None, false);
        kernel.replace_all(if withdraw { Vec::new() } else { vec![replacement] }).unwrap();
        let error = initial_context(&kernel, &compiled, None).await.unwrap_err();
        assert!(matches!(error, NomiPluginToolError::Kernel(
            KernelError::CapabilityProvenanceDrift { .. } | KernelError::CapabilityNotMaterialized { .. }
        )), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn managed_context_failure_and_oversize_fail_the_canonical_context_consumer() {
    for fail in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let value = StrictJsonValue(json!({"instructions": "x".repeat(64 * 1024)}));
        let registration = managed_context_fixture('a', calls.clone(), Some(value), fail);
        let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
        let materialized = kernel.replace_all(vec![registration]).unwrap();
        let error = initial_context(&kernel, &compile(&materialized), None).await.unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(error.to_string().contains(if fail { "context fixture unavailable" } else { "prompt limit" }), "{error}");
    }
}

#[test]
fn managed_capability_availability_matches_nomi_consumer_shapes() {
    let package = PackageRef { id: PackageId::from(PACKAGE), version: VersionString::from(VERSION) };
    let mut manifest = context_capability(&package);
    assert!(nomifun_ai_agent::supports_nomi_plugin_capability(&manifest));
    manifest.supported_surfaces = capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui]);
    assert!(!nomifun_ai_agent::supports_nomi_plugin_capability(&manifest));
    manifest.supported_surfaces = capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent]);
    manifest.contributions.context_schema_refs.clear();
    assert!(!nomifun_ai_agent::supports_nomi_plugin_capability(&manifest));
    manifest.kind = CapabilityKind::ResourceProvider;
    assert!(!nomifun_ai_agent::supports_nomi_plugin_capability(&manifest));
}

#[tokio::test]
async fn explicitly_admitted_initial_context_uses_the_canonical_prompt() {
    let registration = registration_with_source(
        'd', "bundled:", Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())),
        PluginSourceKind::Bundled, true,
    );
    let kernel = Arc::new(KernelRegistry::new(policy_for(PluginSourceKind::Bundled),
        Arc::new(InMemoryPluginStatePersistence::new()) as Arc<dyn PluginStatePersistence>).unwrap());
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let context_admission = nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
        &materialized, BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]), BTreeSet::new(),
    ).unwrap();
    let (contributions, turn_ids) = initial_context(&kernel, &compile(&materialized), Some(&context_admission)).await.unwrap();
    assert!(turn_ids.is_empty());
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0].capability_id().as_ref(), CONTEXT_CAPABILITY);
    assert_eq!(contributions[0].value().0, json!({"source":"bundled-context-fixture", "role":"assistant"}));
    let prompt = nomifun_ai_agent::render_initial_capability_context_section(&contributions).unwrap().unwrap();
    assert!(prompt.contains("<nomifun_initial_capability_context"));
    assert!(prompt.contains(CONTEXT_CAPABILITY));
    assert!(prompt.contains("bundled-context-fixture"));
    assert!(prompt.ends_with("</nomifun_initial_capability_context>"));
}

#[test]
fn bundled_context_admission_rejects_placeholders_test_fixtures_and_native_duplicates() {
    fn materialize(source_kind: PluginSourceKind, host_port: bool) -> Arc<nomifun_agent_kernel::MaterializedRegistry> {
        let registration = registration_with_source('c', "blocked:", Arc::new(AtomicUsize::new(0)),
            Arc::new(Mutex::new(Vec::new())), source_kind, host_port);
        let kernel = KernelRegistry::new(policy_for(source_kind), Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
        kernel.replace_all(vec![registration]).unwrap()
    }
    for (source, host_port, expected) in [
        (PluginSourceKind::Bundled, false, "no typed host binding"),
        (PluginSourceKind::TestFixture, true, "not an exact bundled PlatformBuiltin"),
    ] {
        let registry = materialize(source, host_port);
        let error = nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
            &registry, BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]), BTreeSet::new(),
        ).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    let registry = materialize(PluginSourceKind::Bundled, true);
    let error = nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
        &registry, BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
        BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
    ).unwrap_err();
    assert!(error.to_string().contains("native context path"));
}

#[tokio::test]
async fn canonical_builtin_discovery_preserves_ranking_and_input_boundaries() {
    let base = registration('a', "unused:", Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let mut registration = PluginRegistration::new(base.metadata);
    registration.metadata.manifest.payload.contributions.capabilities.iter_mut()
        .find(|capability| capability.id.as_ref() == AGENT_TOOL).unwrap()
        .contributions.actions = vec![nomifun_ai_agent::tool_discovery::action()];
    registration.metadata.manifest = ArtifactEnvelope::new(registration.metadata.manifest.payload.clone()).unwrap();
    registration.add_capability_handler(AGENT_TOOL.into(), Arc::new(nomifun_ai_agent::tool_discovery::BuiltinDiscovery)).unwrap();
    registration.add_capability_handler(UI_ONLY_TOOL.into(), Arc::new(CapturingHandler {
        prefix: "unused:", calls: Arc::new(AtomicUsize::new(0)), evidence: Arc::new(Mutex::new(Vec::new())),
    })).unwrap();
    registration.add_capability_context_factory(CONTEXT_CAPABILITY.into(), Arc::new(EmptyContextFactory)).unwrap();
    let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let mut revision = revision_with_capabilities(&registry);
    revision.payload.enabled_capabilities.iter_mut().find(|selection| selection.capability.id.as_ref() == AGENT_TOOL).unwrap()
        .action_allowlist = BTreeSet::from([ActionId::from(nomifun_ai_agent::tool_discovery::ACTION_ID)]);
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let compiled = Arc::new(compile_revision(&registry, revision));
    let active = nomifun_agent_kernel::SessionCapabilityState::new(&compiled).snapshot().unwrap();
    async fn discover(
        kernel: &Arc<KernelRegistry>, compiled: &Arc<nomifun_agent_kernel::CompiledSnapshot>,
        active: &nomifun_agent_kernel::ActiveCapabilitySetSnapshot, input: Value,
    ) -> Result<Value, KernelError> {
        let key = format!("discovery-test:{}", uuid::Uuid::now_v7());
        kernel.invoke_shared(compiled.clone(), active, nomifun_agent_kernel::CapabilityInvocationRequest {
            principal: owner(), session_owner: owner(), agent_session_id: SESSION.into(),
            turn_id: "turn-discovery-test".into(), operation_id: key.clone().into(),
            idempotency_key: key.clone().into(), correlation_id: key.into(),
            resolved_snapshot_ref: compiled.snapshot_ref().clone(), active_set_generation: active.generation,
            capability_id: AGENT_TOOL.into(), action_id: nomifun_ai_agent::tool_discovery::ACTION_ID.into(),
            resource_binding_ids: BTreeSet::new(), state_scope_key: format!("session:{SESSION}").into(),
            input: StrictJsonValue(input),
        }).await.map(|value| value.0)
    }
    let candidates = json!([
        {"name":"Alpha", "description":"", "aliases":[]},
        {"name":"AliasHolder", "description":"", "aliases":["alpha", "special"]},
        {"name":"Alphabet", "description":"", "aliases":[]},
    ]);
    for (query, expected) in [(" ALPHA ", json!(["Alpha"])), ("special", json!(["AliasHolder"])), ("missing", json!([]))] {
        let output = discover(&kernel, &compiled, &active, json!({"query":query,"candidates":candidates,"limit":5})).await.unwrap();
        assert_eq!(output["names"], expected);
    }
    let ranked = json!([
        {"name":"Description", "description":"find", "aliases":[]},
        {"name":"ContainsAlias", "description":"", "aliases":["x-find-y"]},
        {"name":"something_find", "description":"", "aliases":[]},
        {"name":"Alias", "description":"", "aliases":["find-here"]},
        {"name":"find-z", "description":"", "aliases":[]},
        {"name":"find-a", "description":"", "aliases":[]},
    ]);
    for (limit, expected) in [(5, json!(["find-a","find-z","Alias","something_find","ContainsAlias"])), (2, json!(["find-a","find-z"]))] {
        let output = discover(&kernel, &compiled, &active, json!({"query":"find","candidates":ranked,"limit":limit})).await.unwrap();
        assert_eq!(output["names"], expected);
    }
    for input in [
        json!({"query":"", "candidates":[], "limit":5}),
        json!({"query":"   ", "candidates":[], "limit":5}),
        json!({"query":"find", "candidates":ranked, "limit":0}),
        json!({"query":"find", "candidates":ranked, "limit":6}),
        json!({"query":"a", "candidates":[{"name":"Alpha","description":"", "aliases":[]}], "limit":5}),
    ] {
        assert!(discover(&kernel, &compiled, &active, input).await.is_err());
    }
    for candidates in [
        json!([{"name":"a", "description":"", "aliases":[]}]),
        json!([{"name":"ShortAlias", "description":"", "aliases":["a"]}]),
    ] {
        let output = discover(&kernel, &compiled, &active, json!({"query":"a","candidates":candidates,"limit":5})).await.unwrap();
        assert_eq!(output["names"].as_array().unwrap().len(), 1);
    }
}
