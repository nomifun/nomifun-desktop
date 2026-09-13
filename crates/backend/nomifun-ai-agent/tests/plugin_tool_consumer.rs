use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use nomi_agent::tool_execution::{ProviderToolAuthority, execute_tool_calls_scoped};
use nomi_protocol::events::ToolCategory;
use nomi_tools::{
    Tool, ToolExecutionContext,
    registry::ToolRegistry,
};
use nomi_types::tool::{JsonSchema, ToolResult};
use nomi_types::message::ContentBlock;
use nomifun_agent_contracts::{
    ActionId, AgentPresetId, AgentPresetRevision, ArtifactId,
    AgentPresetRevisionPayload, AgentSessionId, ArtifactEnvelope,
    CancellationDescriptor, CapabilityActionDescriptor,
    CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind, CapabilityManifest, CapabilityRef,
    CapabilitySelection, CanonicalSchemaRef, ContributionLock,
    ContributionSourceKind,
    DeclaredServiceViewDescriptor, DigestHex, EffectClass, HostPortId,
    HostPortBindingDescriptor, HostPortRef, InProcessEntrypointMetadata,
    LocalizedMetadata,
    ManagedTaskRegistrationDescriptor, OperationId, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint,
    PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor,
    MiniAppId, MiniAppReleaseId, MiniAppReleaseRef, PluginMountId,
    PluginRegistrarDescriptor, PluginRegistrarOperation,
    PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateHandleDescriptor, PluginStateMethod, PresetRevisionRef,
    PrincipalRef, RuntimeProfileKind, RuntimeTarget, ScopeKey,
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
    KernelNomiPluginToolSession, NomiHostDynamicToolDescriptor,
    NomiHostDynamicToolError, NomiHostDynamicToolInvocation,
    NomiHostDynamicToolInvoker, NomiMiniAppToolInvoker,
    NomiMiniAppToolInvocation, NomiMiniAppToolSchemaResolver,
    NomiPlatformBuiltinToolAdmission,
    NomiPlatformBuiltinToolSchemaResolver, NomiPluginToolError,
    NomiPluginToolSchemaResolver,
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

#[derive(Default)]
struct SchemaMap {
    schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
}

#[derive(Default)]
struct MiniAppSchemaMap {
    schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
}

#[async_trait]
impl NomiMiniAppToolSchemaResolver for MiniAppSchemaMap {
    async fn resolve(
        &self,
        _owner: &PrincipalRef,
        _capability: &nomifun_agent_contracts::ResolvedMiniAppCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        self.schemas
            .get(reference)
            .cloned()
            .ok_or_else(|| format!("MiniApp schema {} is missing", reference.as_ref()))
    }
}

#[derive(Default)]
struct CapturingMiniAppInvoker {
    calls: AtomicUsize,
    action_ids: Mutex<Vec<String>>,
}

#[derive(Default)]
struct CountingHostDynamicInvoker {
    calls: AtomicUsize,
}

#[async_trait]
impl NomiHostDynamicToolInvoker for CountingHostDynamicInvoker {
    async fn invoke(
        &self,
        _request: NomiHostDynamicToolInvocation,
    ) -> Result<StrictJsonValue, NomiHostDynamicToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(StrictJsonValue(json!({"accepted": true})))
    }
}

#[async_trait]
impl NomiMiniAppToolInvoker for CapturingMiniAppInvoker {
    async fn invoke(
        &self,
        request: NomiMiniAppToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.action_ids
            .lock()
            .unwrap()
            .push(request.action().action_id.as_ref().to_owned());
        Ok(StrictJsonValue(json!({
            "miniapp": request.capability().miniapp_id.as_ref(),
            "action": request.action().action_id.as_ref(),
            "input": request.input().0,
        })))
    }
}

#[async_trait]
impl NomiPluginToolSchemaResolver for SchemaMap {
    async fn resolve(
        &self,
        _capability: &nomifun_agent_contracts::ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        self.schemas
            .get(reference)
            .cloned()
            .ok_or_else(|| format!("schema {} is missing", reference.as_ref()))
    }
}

#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for SchemaMap {
    async fn resolve(
        &self,
        _capability: &nomifun_agent_contracts::ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        self.schemas
            .get(reference)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "bundled schema {} is missing",
                    reference.as_ref()
                )
            })
    }
}

struct NativeSentinel;

#[async_trait]
impl Tool for NativeSentinel {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Native regression sentinel."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::text("native-ok")
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
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
        version: VersionString::from(VERSION),
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
        version: VersionString::from(VERSION),
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
) -> (PluginRegistration, Arc<SchemaMap>) {
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
) -> (PluginRegistration, Arc<SchemaMap>) {
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
    let mount_id = PluginMountId::from(MOUNT);
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
            Arc::new(EmptyContextFactory),
        )
        .unwrap();

    let schemas = Arc::new(SchemaMap {
        schemas: BTreeMap::from([(
            agent_action.input_schema,
            StrictJsonValue(input_schema),
        )]),
    });
    (registration, schemas)
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
            version: VersionString::from(VERSION),
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
    let ui = selection(UI_ONLY_TOOL, &[UI_ONLY_ACTION]);
    let context = selection(CONTEXT_CAPABILITY, &[]);
    let enabled_capabilities = vec![ui, tool, context];
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from(
                "0190f5fe-7c00-7a00-8000-000000000004",
            ),
            revision: 1,
            revision_digest: DigestHex::from(""),
        },
        payload: AgentPresetRevisionPayload {
            schema_version: VersionString::from(VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities,
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
        },
        contribution_locks: [AGENT_TOOL, UI_ONLY_TOOL, CONTEXT_CAPABILITY]
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
            miniapp_capabilities: Vec::new(),
            revision: revision_with_capabilities(
                materialized,
            ),
            principal: owner(),
            scene: "chat".to_owned(),
            surface: "desktop".to_owned(),
            audience: "owner".to_owned(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("plugin-tool-test-resolve"),
        },
    )
    .unwrap()
}

async fn session(
    kernel: Arc<KernelRegistry>,
    compiled: nomifun_agent_kernel::CompiledSnapshot,
    schemas: Arc<SchemaMap>,
) -> nomifun_ai_agent::NomiPluginToolSession {
    KernelNomiPluginToolSession::materialize(
        kernel,
        Arc::new(compiled),
        owner(),
        AgentSessionId::from(SESSION),
        ScopeKey::from(format!("session:{SESSION}")),
        schemas,
    )
    .await
    .unwrap()
}

async fn session_with_platform_builtins(
    kernel: Arc<KernelRegistry>,
    compiled: nomifun_agent_kernel::CompiledSnapshot,
    schemas: Arc<SchemaMap>,
    admission: Arc<NomiPlatformBuiltinToolAdmission>,
) -> nomifun_ai_agent::NomiPluginToolSession {
    let plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver> =
        schemas;
    KernelNomiPluginToolSession::materialize_with_platform_builtins(
        kernel,
        Arc::new(compiled),
        owner(),
        AgentSessionId::from(SESSION),
        ScopeKey::from(format!("session:{SESSION}")),
        plugin_schema_resolver,
        admission,
    )
    .await
    .unwrap()
}

async fn session_with_platform_builtins_and_context(
    kernel: Arc<KernelRegistry>,
    compiled: nomifun_agent_kernel::CompiledSnapshot,
    schemas: Arc<SchemaMap>,
    tool_admission: Arc<NomiPlatformBuiltinToolAdmission>,
    context_admission: Arc<
        nomifun_ai_agent::NomiPlatformBuiltinContextAdmission,
    >,
) -> nomifun_ai_agent::NomiPluginToolSession {
    let plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver> =
        schemas;
    KernelNomiPluginToolSession::materialize_with_platform_builtins_and_context(
        kernel,
        Arc::new(compiled),
        owner(),
        AgentSessionId::from(SESSION),
        ScopeKey::from(format!("session:{SESSION}")),
        plugin_schema_resolver,
        tool_admission,
        context_admission,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn dynamic_schema_registers_and_kernel_invoke_preserves_native_tools() {
    let calls = Arc::new(AtomicUsize::new(0));
    let evidence = Arc::new(Mutex::new(Vec::new()));
    let (registration, schemas) =
        registration('a', "first:", Arc::clone(&calls), Arc::clone(&evidence));
    let kernel = Arc::new(
        KernelRegistry::new(
            policy(),
            Arc::new(InMemoryPluginStatePersistence::new())
                as Arc<dyn PluginStatePersistence>,
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let session = session(
        Arc::clone(&kernel),
        compile(&materialized),
        schemas,
    )
    .await;

    assert_eq!(session.actions().len(), 1);
    let action = &session.actions()[0];
    assert_eq!(action.capability_id().as_ref(), AGENT_TOOL);
    assert_eq!(action.action_id().as_ref(), AGENT_ACTION);
    assert!(action.provider_name().starts_with("plugin__"));
    assert!(action.provider_name().len() <= 64);
    assert!(action.activation_identity().contains(AGENT_TOOL));
    assert!(action.activation_identity().contains(AGENT_ACTION));
    assert!(action.activation_identity().contains(&"a".repeat(64)));
    assert_eq!(
        action.artifact_identity(),
        format!("{AGENT_TOOL} {AGENT_ACTION}")
    );
    assert!(
        nomi_agent::output::artifact_contract(action.artifact_identity())
            .is_none(),
        "ordinary Plugin Tools must not inherit artifact obligations from provenance JSON"
    );
    assert!(
        nomi_agent::output::artifact_contract(action.activation_identity())
            .is_some(),
        "regression fixture must demonstrate why canonical activation JSON is unsafe for artifact classification"
    );

    let mut registry = ToolRegistry::new();
    assert!(registry.register(Box::new(NativeSentinel)));
    let mut allowed = vec!["Read".to_owned()];
    let mut deferred = Vec::new();
    session.extend_tool_policy(&mut allowed, &mut deferred);
    registry.retain_only_named(&allowed);
    session.register_into(&mut registry).unwrap();

    assert!(registry.get("Read").is_some());
    let definition = registry
        .to_tool_defs()
        .into_iter()
        .find(|definition| definition.name == action.provider_name())
        .unwrap();
    assert_eq!(definition.input_schema, action.input_schema().0);
    assert!(!definition.deferred);

    let native = registry.get("Read").unwrap().execute(json!({})).await;
    assert!(!native.is_error);
    assert_eq!(native.content, "native-ok");

    let execution_context =
        ToolExecutionContext::from_scoped_tool_call("turn-1", "call-1");
    let result = registry
        .get(action.provider_name())
        .unwrap()
        .execute_with_context(
            json!({"message": "hello"}),
            &execution_context,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["echo"],
        "first:hello"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let evidence = evidence.lock().unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].agent_session_id, SESSION);
    assert_eq!(evidence[0].capability_id, AGENT_TOOL);
    assert_eq!(evidence[0].action_id, AGENT_ACTION);
    assert_eq!(evidence[0].target_artifact_digest, "a".repeat(64));
    assert_eq!(evidence[0].operation_id, evidence[0].idempotency_key);
    assert_eq!(evidence[0].operation_id, evidence[0].correlation_id);
    assert!(evidence[0].operation_id.starts_with("nomi-plugin:tool-call-v1-"));
}

#[tokio::test]
async fn invalid_host_dynamic_payload_is_rejected_before_invoker_dispatch() {
    let kernel_calls = Arc::new(AtomicUsize::new(0));
    let (registration, schemas) = registration(
        'a',
        "base:",
        Arc::clone(&kernel_calls),
        Arc::new(Mutex::new(Vec::new())),
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy(),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let base = session(kernel, compile(&materialized), schemas).await;
    let dynamic_invoker = Arc::new(CountingHostDynamicInvoker::default());
    let dynamic_name = "robot_dynamic_schema_guard";
    let session = base
        .with_host_dynamic_tools(
            vec![NomiHostDynamicToolDescriptor {
                capability_id: CapabilityId::from("robot.display"),
                provider_name: dynamic_name.to_owned(),
                description: "Schema-guarded host dynamic Tool".to_owned(),
                input_schema: StrictJsonValue(json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "expression": {"type": "string", "minLength": 1}
                    },
                    "required": ["expression"]
                })),
                effect_class: EffectClass::Physical,
                deferred: false,
            }],
            dynamic_invoker.clone(),
        )
        .unwrap();
    let mut registry = ToolRegistry::new();
    session.register_into(&mut registry).unwrap();
    let authority = ProviderToolAuthority::from_request_tools(&registry.to_tool_defs());

    let outcome = execute_tool_calls_scoped(
        &registry,
        &[ContentBlock::ToolUse {
            id: "invalid-dynamic-call".to_owned(),
            name: dynamic_name.to_owned(),
            input: json!({"expression": 7, "credential": "sk-must-not-dispatch"}),
            extra: None,
        }],
        &authority,
        "invalid-dynamic-turn",
        None,
        Default::default(),
        false,
    )
    .await
    .unwrap();

    assert!(matches!(
        &outcome.results[0],
        ContentBlock::ToolResult { is_error: true, content, .. }
            if content.contains("the tool was not executed")
                && !content.contains("sk-must-not-dispatch")
    ));
    assert_eq!(dynamic_invoker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(kernel_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicitly_admitted_bundled_tool_invokes_the_exact_kernel_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let evidence = Arc::new(Mutex::new(Vec::new()));
    let (registration, schemas) = registration_with_source(
        'b',
        "bundled:",
        Arc::clone(&calls),
        Arc::clone(&evidence),
        PluginSourceKind::Bundled,
        true,
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::Bundled),
            Arc::new(InMemoryPluginStatePersistence::new())
                as Arc<dyn PluginStatePersistence>,
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let compiled = compile(&materialized);

    let plugin_only = session(
        Arc::clone(&kernel),
        compiled.clone(),
        Arc::clone(&schemas),
    )
    .await;
    assert!(
        plugin_only.actions().is_empty(),
        "the legacy PluginMount-only entrypoint must not implicitly expose bundled tools"
    );

    let builtin_schema_resolver: Arc<
        dyn NomiPlatformBuiltinToolSchemaResolver,
    > = schemas.clone();
    let admission = Arc::new(
        NomiPlatformBuiltinToolAdmission::from_registry(
            &materialized,
            BTreeSet::from([CapabilityId::from(AGENT_TOOL)]),
            BTreeSet::new(),
            builtin_schema_resolver,
        )
        .unwrap(),
    );
    assert_eq!(
        admission.approved_capability_ids(),
        BTreeSet::from([CapabilityId::from(AGENT_TOOL)])
    );
    let session = session_with_platform_builtins(
        Arc::clone(&kernel),
        compiled,
        schemas,
        admission,
    )
    .await;
    assert_eq!(session.actions().len(), 1);

    let action = &session.actions()[0];
    let mut registry = ToolRegistry::new();
    let mut allowed = Vec::new();
    let mut deferred = Vec::new();
    session.extend_tool_policy(&mut allowed, &mut deferred);
    registry.retain_only_named(&allowed);
    session.register_into(&mut registry).unwrap();
    let result = registry
        .get(action.provider_name())
        .unwrap()
        .execute_with_context(
            json!({"message": "hello"}),
            &ToolExecutionContext::from_scoped_tool_call(
                "turn-bundled",
                "call-bundled",
            ),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["echo"],
        "bundled:hello"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let evidence = evidence.lock().unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].capability_id, AGENT_TOOL);
    assert_eq!(
        evidence[0].target_artifact_digest,
        materialized
            .capability(&CapabilityId::from(AGENT_TOOL))
            .unwrap()
            .target_artifact_digest
            .as_ref()
    );
}

#[tokio::test]
async fn explicitly_admitted_initial_context_uses_the_same_snapshot_and_prompt() {
    let (registration, schemas) = registration_with_source(
        'd',
        "bundled:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
        PluginSourceKind::Bundled,
        true,
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::Bundled),
            Arc::new(InMemoryPluginStatePersistence::new())
                as Arc<dyn PluginStatePersistence>,
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver> =
        schemas.clone();
    let tool_admission = Arc::new(
        NomiPlatformBuiltinToolAdmission::from_registry(
            &materialized,
            BTreeSet::new(),
            BTreeSet::new(),
            resolver,
        )
        .unwrap(),
    );
    let context_admission = Arc::new(
        nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
            &materialized,
            BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
            BTreeSet::new(),
        )
        .unwrap(),
    );
    let session = session_with_platform_builtins_and_context(
        kernel,
        compile(&materialized),
        schemas,
        tool_admission,
        context_admission,
    )
    .await;

    assert!(session.actions().is_empty());
    assert_eq!(session.initial_context_contributions().len(), 1);
    let contribution = &session.initial_context_contributions()[0];
    assert_eq!(contribution.capability_id().as_ref(), CONTEXT_CAPABILITY);
    assert_eq!(
        contribution.value().0,
        json!({
            "source": "bundled-context-fixture",
            "role": "assistant"
        })
    );
    let prompt = session
        .system_prompt_with_initial_context(Some("base instructions"))
        .unwrap()
        .unwrap();
    assert!(prompt.starts_with("base instructions\n\n"));
    assert!(prompt.contains("<nomifun_initial_capability_context"));
    assert!(prompt.contains(CONTEXT_CAPABILITY));
    assert!(prompt.contains("bundled-context-fixture"));
    assert!(prompt.ends_with("</nomifun_initial_capability_context>"));
}







#[test]
fn bundled_admission_rejects_placeholders_test_fixtures_and_native_duplicates() {
    fn materialize(
        source_kind: PluginSourceKind,
        declare_capability_host_port: bool,
    ) -> (
        Arc<nomifun_agent_kernel::MaterializedRegistry>,
        Arc<SchemaMap>,
    ) {
        let (registration, schemas) = registration_with_source(
            'c',
            "blocked:",
            Arc::new(AtomicUsize::new(0)),
            Arc::new(Mutex::new(Vec::new())),
            source_kind,
            declare_capability_host_port,
        );
        let kernel = KernelRegistry::new(
            policy_for(source_kind),
            Arc::new(InMemoryPluginStatePersistence::new())
                as Arc<dyn PluginStatePersistence>,
        )
        .unwrap();
        (kernel.replace_all(vec![registration]).unwrap(), schemas)
    }

    let (placeholder, schemas) =
        materialize(PluginSourceKind::Bundled, false);
    let resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver> = schemas;
    let error = NomiPlatformBuiltinToolAdmission::from_registry(
        &placeholder,
        BTreeSet::from([CapabilityId::from(AGENT_TOOL)]),
        BTreeSet::new(),
        resolver,
    )
    .expect_err("metadata-only bundled registrations must not be admitted");
    assert!(error.to_string().contains("no typed capability host port"));
    let error = nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
        &placeholder,
        BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
        BTreeSet::new(),
    )
    .expect_err("metadata-only bundled context must not be admitted");
    assert!(error.to_string().contains("no typed host binding"));

    let (fixture, schemas) =
        materialize(PluginSourceKind::TestFixture, true);
    let resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver> = schemas;
    let error = NomiPlatformBuiltinToolAdmission::from_registry(
        &fixture,
        BTreeSet::from([CapabilityId::from(AGENT_TOOL)]),
        BTreeSet::new(),
        resolver,
    )
    .expect_err("TestFixture registrations must not be admitted");
    assert!(error.to_string().contains("not an exact bundled PlatformBuiltin"));

    let (bundled, schemas) = materialize(PluginSourceKind::Bundled, true);
    let resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver> = schemas;
    let error = NomiPlatformBuiltinToolAdmission::from_registry(
        &bundled,
        BTreeSet::from([CapabilityId::from(AGENT_TOOL)]),
        BTreeSet::from([CapabilityId::from(AGENT_TOOL)]),
        resolver,
    )
    .expect_err("native and Kernel Tool ownership must be disjoint");
    assert!(error.to_string().contains("also owned by Nomi's native registry"));

    let error = nomifun_ai_agent::NomiPlatformBuiltinContextAdmission::from_registry(
        &bundled,
        BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
        BTreeSet::from([CapabilityId::from(CONTEXT_CAPABILITY)]),
    )
    .expect_err("native and Kernel context ownership must be disjoint");
    assert!(error.to_string().contains("native context path"));
}

#[tokio::test]
async fn non_agent_tool_non_tool_and_hidden_actions_never_register() {
    let (registration, schemas) = registration(
        'a',
        "filter:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy(),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let compiled = compile(&materialized);
    assert!(compiled
        .content()
        .enabled_capabilities
        .iter()
        .any(|capability| capability.capability.id.as_ref() == UI_ONLY_TOOL));
    assert!(compiled
        .content()
        .enabled_capabilities
        .iter()
        .any(|capability| {
            capability.capability.id.as_ref() == CONTEXT_CAPABILITY
        }));

    let session = session(kernel, compiled, schemas).await;
    assert_eq!(session.actions().len(), 1);
    assert_eq!(session.actions()[0].capability_id().as_ref(), AGENT_TOOL);
    assert_eq!(session.actions()[0].action_id().as_ref(), AGENT_ACTION);
}

#[tokio::test]
async fn stale_artifact_fails_before_replacement_handler_dispatch() {
    let original_calls = Arc::new(AtomicUsize::new(0));
    let replacement_calls = Arc::new(AtomicUsize::new(0));
    let (original, schemas) = registration(
        'a',
        "original:",
        Arc::clone(&original_calls),
        Arc::new(Mutex::new(Vec::new())),
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy(),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let materialized = kernel.replace_all(vec![original]).unwrap();
    let session = session(
        Arc::clone(&kernel),
        compile(&materialized),
        schemas,
    )
    .await;

    let (replacement, _) = registration(
        'b',
        "replacement:",
        Arc::clone(&replacement_calls),
        Arc::new(Mutex::new(Vec::new())),
    );
    kernel.replace_all(vec![replacement]).unwrap();

    let mut registry = ToolRegistry::new();
    session.register_into(&mut registry).unwrap();
    let action = &session.actions()[0];
    let result = registry
        .get(action.provider_name())
        .unwrap()
        .execute_with_context(
            json!({"message": "must-not-run"}),
            &ToolExecutionContext::from_scoped_tool_call(
                "turn-stale",
                "call-stale",
            ),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("CAPABILITY_NOT_MATERIALIZED"));
    assert!(!result.content.contains("provenance"));
    assert_eq!(original_calls.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_calls.load(Ordering::SeqCst), 0);
}



fn miniapp_fixture() -> (
    nomifun_agent_contracts::ResolvedMiniAppCapability,
    Arc<MiniAppSchemaMap>,
) {
    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {"type": "string"}
        },
        "required": ["message"]
    });
    let output_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "miniapp": {"type": "string"},
            "action": {"type": "string"},
            "input": {"type": "object"}
        },
        "required": ["miniapp", "action", "input"]
    });
    let input_ref = schema_ref("miniapp.fixture/input", &input_schema);
    let output_ref = schema_ref("miniapp.fixture/output", &output_schema);
    let action = CapabilityActionDescriptor {
        action_id: ActionId::from("miniapp.fixture.echo.invoke"),
        input_schema: input_ref.clone(),
        output_schema: output_ref,
        effect_class: EffectClass::Pure,
        presentation: ToolPresentationKind::FunctionTool,
    };
    let miniapp_id = MiniAppId::from("miniapp-fixture");
    let capability_id = CapabilityId::from("miniapp.fixture.echo");
    let contribution_id =
        nomifun_agent_contracts::ContributionId::from("capability:miniapp.fixture.echo");
    let capability = nomifun_agent_contracts::ResolvedMiniAppCapability {
        capability: CapabilityRef {
            id: capability_id,
            version: VersionString::from(VERSION),
        },
        source_package: PackageRef {
            id: PackageId::from("miniapp.fixture"),
            version: VersionString::from(VERSION),
        },
        contribution_id: contribution_id.clone(),
        contribution_lock: ContributionLock {
            source_kind: ContributionSourceKind::MiniAppActiveRelease,
            source_identity: format!("miniapp:{}", miniapp_id.as_ref()).into(),
            mount_id: None,
            miniapp_id: Some(miniapp_id.clone()),
            mcp_binding_id: None,
            contribution_id,
            contract_digest: DigestHex::from("a".repeat(64)),
        },
        miniapp_id: miniapp_id.clone(),
        active_release: MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from("release-fixture"),
            artifact_id: ArtifactId::from("artifact-fixture"),
            release_digest: DigestHex::from("b".repeat(64)),
            manifest_digest: DigestHex::from("c".repeat(64)),
        },
        active_release_epoch: 3,
        catalog_digest: DigestHex::from("d".repeat(64)),
        display_name: "MiniApp Fixture".to_owned(),
        description: "MiniApp fixture action".to_owned(),
        actions: vec![action],
        required_resource_kinds: BTreeSet::new(),
        action_allowlist: BTreeSet::new(),
    };
    let schemas = Arc::new(MiniAppSchemaMap {
        schemas: BTreeMap::from([(input_ref, StrictJsonValue(input_schema))]),
    });
    (capability, schemas)
}

fn compile_miniapp_fixture(
    capability: &nomifun_agent_contracts::ResolvedMiniAppCapability,
) -> nomifun_agent_kernel::CompiledSnapshot {
    let selection = CapabilitySelection {
        capability: capability.capability.clone(),
        action_allowlist: capability.action_allowlist.clone(),
    };
    let payload = AgentPresetRevisionPayload {
        schema_version: VersionString::from(VERSION),
        model_route_refs: BTreeMap::new(),
        chat_route_records: BTreeMap::new(),
        enabled_capabilities: vec![selection],

        skill_bindings: Vec::new(),
        system_role_provider_overrides: BTreeMap::new(),
        persona: "MiniApp fixture".to_owned(),
        instructions: "Use the MiniApp fixture.".to_owned(),
        starter_prompts: Vec::new(),
    };
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from("miniapp.fixture.preset"),
            revision: 1,
            revision_digest: DigestHex::from(""),
        },
        payload,
        contribution_locks: vec![capability.contribution_lock.clone()],
        created_by: UserId::from(OWNER),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    AgentPresetCompiler::compile(
        &nomifun_agent_kernel::MaterializedRegistry::empty(),
        &CompilerEnvironment {
            resolver_version: VersionString::from(VERSION),
            required_runtime_protocol_version: VersionString::from(VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("1".repeat(64)),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from("2".repeat(64)),
            target_contribution_manifest_digest: DigestHex::from("3".repeat(64)),
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: "miniapp-tool-test".to_owned(),
        },
        CompileRequest {
            miniapp_capabilities: vec![capability.clone()],
            revision,
            principal: owner(),
            scene: "chat".to_owned(),
            surface: "desktop".to_owned(),
            audience: "owner".to_owned(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("miniapp-tool-test"),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn miniapp_active_release_action_joins_the_same_nomi_tool_session() {
    let (capability, schemas) = miniapp_fixture();
    let compiled = compile_miniapp_fixture(&capability);
    let kernel = Arc::new(
        KernelRegistry::new(
            policy(),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let base = KernelNomiPluginToolSession::materialize(
        kernel,
        Arc::new(compiled.clone()),
        owner(),
        AgentSessionId::from(SESSION),
        ScopeKey::from(format!("session:{SESSION}")),
        Arc::new(SchemaMap::default()),
    )
    .await
    .unwrap();
    let actions = KernelNomiPluginToolSession::materialize_miniapp_actions(
        &compiled,
        &owner(),
        &AgentSessionId::from(SESSION),
        &ScopeKey::from(format!("session:{SESSION}")),
        schemas,
    )
    .await
    .unwrap();
    assert_eq!(actions.len(), 1);
    assert!(actions[0].provider_name().starts_with("miniapp__"));

    let invoker = Arc::new(CapturingMiniAppInvoker::default());
    let session = base
        .with_miniapp_actions(actions, invoker.clone())
        .unwrap();
    assert_eq!(session.actions().len(), 0);
    assert_eq!(session.miniapp_actions().len(), 1);

    let mut registry = ToolRegistry::new();
    session.register_into(&mut registry).unwrap();
    let action = &session.miniapp_actions()[0];
    assert_eq!(
        action.artifact_identity(),
        "miniapp.fixture.echo miniapp.fixture.echo.invoke"
    );
    assert!(
        nomi_agent::output::artifact_contract(action.artifact_identity())
            .is_none(),
        "ordinary MiniApp Tools must not inherit artifact obligations from release provenance"
    );
    let result = registry
        .get(action.provider_name())
        .unwrap()
        .execute_with_context(
            json!({"message": "hello"}),
            &ToolExecutionContext::from_scoped_tool_call(
                "turn-miniapp",
                "call-miniapp",
            ),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let output: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(output["miniapp"], "miniapp-fixture");
    assert_eq!(output["action"], "miniapp.fixture.echo.invoke");
    assert_eq!(invoker.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        invoker.action_ids.lock().unwrap().as_slice(),
        &["miniapp.fixture.echo.invoke".to_owned()]
    );
}
