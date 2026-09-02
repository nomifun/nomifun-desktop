use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, ArtifactEnvelope, ArtifactId, CapabilityActionDescriptor,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest, CapabilityRef,
    CancellationDescriptor, CanonicalErrorCode, CanonicalSchemaRef, ConnectionConfigRef,
    ChatRouteIdentity, CorrelationId, DeclaredServiceViewDescriptor, DeleteAgentSessionCommand,
    DigestHex,
    EffectClass, EventId, EventProducerId, FullAutoExecutionWire, HostPortId, HostPortRef,
    IdempotencyKey, InProcessEntrypointMetadata, LocalizedMetadata, LogicalArtifactRef,
    ManagedTaskRegistrationDescriptor, McpServerId, McpToolCapabilityMapping, McpToolKey,
    ModelRouteId, NativeActionStart, NativeActionStartAck, OperationId, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState, PluginEffectiveState,
    PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateCompareAndSwapOutcome, PluginStateHandleDescriptor,
    PluginStateMethod, PrincipalRef, ResolvedSnapshotEnvelope,
    ResourceBindingId, ResourceKind, RuntimeBindingContract, RuntimeBindingId,
    RuntimeCommand, RuntimeCommandContext, RuntimeCreateParams, RuntimeEventAck,
    RuntimeEventEnvelope, RuntimeFeatureId, RuntimeHelloPayload, RuntimeProfileKind,
    RuntimeSessionDisposeParams, RuntimeStartTurnParams,
    RuntimeTarget, ScopeKey, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef, SkillDefinition, SkillId, StateKey, StrictJsonValue,
    ToolPresentationKind, UserId, ValidatedPluginConfig, VersionString, digest_bytes,
    digest_payload,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, CatalogSnapshot, CompilerReleaseInputs, ControlPlaneError,
    ControlPlaneStore, InMemoryControlPlaneStore, OfficialTemplateCatalog,
    PresetPreviewCompiler, StaticCatalogProvider,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler, CapabilityHandler, CapabilityInvocationContext,
    CapabilityInvocationRequest, CompileRequest, CompiledSnapshot, CompilerEnvironment,
    HostPluginStateApi, InMemoryPluginStatePersistence, KernelError, KernelRegistry,
    MaterializationPolicy, MaterializedRegistry, PluginRegistration, PluginStateError,
    PluginStatePersistence, SessionCapabilityState,
};
use nomifun_agent_session::{
    AgentSessionStore, CreateSessionRequest, EffectEventRequest, EffectStrategy, EffectTerminalState,
    RuntimeAppendContext, SessionStoreError,
};
use nomifun_api_types::{
    AgentPresetDocumentDto, AgentPresetDraftDto, CapabilityExposureDto,
    CapabilitySelectionDto, CreateAgentPresetRequest, EditorDraftStateDto,
    EditorRevisionActionDto, ExactCatalogRefDto, ResolveAgentPresetPreviewRequest,
    TypedResourceBindingDto,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatCausality, ChatCausalityGate,
    ChatContentPart, ChatMessage, ChatModelBroker, ChatModelError, ChatModelErrorCode,
    ChatModelEvent, ChatModelInput, ChatModelRequest, ChatModality, ChatProtocol,
    ChatProtocolAdapter, ChatResponseFormat, ChatRole, ChatRouteResolver, ChatRouteSelection,
    ChatToolCall, ChatToolChoice, ChatToolDefinition, ChatToolResultPart,
    CredentialLease, CredentialTarget, GeminiAdapter, OpenAiChatAdapter,
    OpenAiResponsesAdapter, PromptCachePolicy, ProviderCredentialRef, ProviderCredentialStore,
    ProviderIdRef, ProviderTransport, ProviderWireFrame, ProviderWireRequest,
    ProviderWireStream, ResolvedChatRoute, ResolvedChatRouteSet, ToolCallId, VertexAdapter,
    protocol_features,
};
use nomifun_codex_runtime::{
    ClientLimits, CodexRuntimeSupervisor, DisposeRpcOutcome, InheritedHandleCredential,
    ManagedRuntimeSession, PinnedRuntimeProfile, RuntimeError, RuntimeIngressPort,
    RuntimeLaunchRequest, RuntimeProcessConfig, RuntimeReleaseDescriptor,
};
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FreshV4Coordinator, FreshV4RootError,
    canonical_schema_manifest_digest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

const VERSION: &str = "1.0.0";
const SAMPLE_PACKAGE: &str = "sample.echo";
const SAMPLE_MOUNT: &str = "sample-echo";
const SAMPLE_CAPABILITY: &str = "sample.echo";
const SAMPLE_ACTION: &str = "sample.echo.invoke";
const SAMPLE_SKILL: &str = "sample.echo-guidance";
const SAMPLE_SERVER: &str = "sample.echo.server";
const SAMPLE_RESOURCE_KIND: &str = "sample.echo.target";
const SAMPLE_RESOURCE_BINDING: &str = "sample-echo-target";
const SAMPLE_MODEL_ROUTE: &str = "sample-echo-recorded";
const BUILD_IDENTITY: &str = "c6-sample-echo-2026-08-29";
const RUNTIME_SCRIPT_FILE: &str = "app-server";
const RUNTIME_CONFIG_FILE: &str = "runtime-fixture.json";

const RECORDED_RUNTIME_SCRIPT: &str = r#"
const fs = require("fs");
const readline = require("readline");
const config = JSON.parse(fs.readFileSync("runtime-fixture.json", "utf8"));
const pending = new Map();
const send = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });

lines.on("line", (line) => {
  if (!line.trim()) return;
  const message = JSON.parse(line);
  if (!message.method) {
    const wait = pending.get(String(message.id));
    if (!wait) return;
    pending.delete(String(message.id));
    if (message.error) {
      send({ id: wait.client_id, error: message.error });
      return;
    }
    send({ id: wait.client_id, result: wait.result });
    return;
  }

  switch (message.method) {
    case "runtime/hello":
      send({ id: message.id, result: config.hello });
      break;
    case "create": {
      const serverId = "runtime-bound:" + message.id;
      pending.set(serverId, { client_id: message.id, result: config.binding });
      send({ id: serverId, method: "runtime/event", params: config.runtime_bound_event });
      break;
    }
    case "start_turn": {
      const context = message.params.context;
      const operation = context.operation_id;
      const serverId = "native-action:" + operation;
      const start = {
        agent_session_id: context.agent_session_id,
        runtime_binding_id: context.runtime_binding_id,
        turn_operation_id: operation,
        action_id: config.action.action_id,
        effect_id: "effect:" + operation,
        idempotency_key: "native-action:" + operation,
        capability_id: config.action.capability_id,
        active_set_generation: context.active_set_generation,
        snapshot_digest: context.resolved_snapshot_ref.snapshot_digest,
        resource_binding_ids: config.action.resource_binding_ids
      };
      pending.set(serverId, { client_id: message.id, result: { accepted: true } });
      send({ id: serverId, method: "native_action/start", params: start });
      break;
    }
    case "cancel":
      send({ id: message.id, result: {} });
      break;
    case "session_dispose":
      if (config.dispose_mode === "timeout") break;
      send({
        id: message.id,
        result: {
          agent_session_id: message.params.agent_session_id,
          runtime_binding_id: message.params.runtime_binding_id,
          disposed: true
        }
      });
      setTimeout(() => process.exit(0), 10);
      break;
    default:
      send({ id: message.id, error: { code: -32601, message: "method not found" } });
  }
});
"#;

#[derive(Debug, Error)]
pub enum SampleEchoGateError {
    #[error("sample.echo invariant failed: {0}")]
    Invariant(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    FreshRoot(#[from] FreshV4RootError),
    #[error(transparent)]
    ControlPlane(#[from] ControlPlaneError),
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error(transparent)]
    PluginState(#[from] PluginStateError),
    #[error(transparent)]
    Session(#[from] SessionStoreError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("chat model broker failed ({code:?}): {message}")]
    Broker {
        code: ChatModelErrorCode,
        message: String,
    },
    #[error("canonical digest failed: {0}")]
    Digest(String),
    #[error("sample.echo capability task failed: {0}")]
    Join(String),
}

impl From<ChatModelError> for SampleEchoGateError {
    fn from(error: ChatModelError) -> Self {
        Self::Broker {
            code: error.code,
            message: error.message,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SampleEchoGateConfig {
    pub working_root: PathBuf,
    pub node_executable: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleEchoFaultReport {
    pub save_failure_created_revision: bool,
    pub save_failure_created_session: bool,
    pub materialization_failure_published_generation: bool,
    pub panic_effect_became_uncertain: bool,
    pub panic_retried_effect: bool,
    pub dispose_timeout_forced_tree_cleanup: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleEchoSessionReport {
    pub agent_session_id: String,
    pub revision: u64,
    pub persistent_session: bool,
    pub effect_success_count: u64,
    pub event_count_before_delete: usize,
    pub dispose_rpc: String,
    pub tombstone_committed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleEchoGateReport {
    pub package_materialized: bool,
    pub capability_materialized: bool,
    pub skill_materialized: bool,
    pub mcp_materialized: bool,
    pub config_validated: bool,
    pub clean_revision_action: String,
    pub dirty_revision_action: String,
    pub clean_revision: u64,
    pub dirty_revision: u64,
    pub clean_session: SampleEchoSessionReport,
    pub dirty_session: SampleEchoSessionReport,
    pub broker_recorded_transport_calls: usize,
    pub first_echo: String,
    pub restart_echo: String,
    pub plugin_state_cas_conflict: bool,
    pub plugin_state_survived_restart: bool,
    pub plugin_state_survived_session_delete: bool,
    pub faults: SampleEchoFaultReport,
}

#[derive(Default)]
struct EchoControl {
    panic_next: AtomicBool,
    successful_effects: AtomicU64,
}

struct EchoHandler {
    prefix: String,
    control: Arc<EchoControl>,
}

#[async_trait]
impl CapabilityHandler for EchoHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        if self.control.panic_next.swap(false, Ordering::AcqRel) {
            panic!("sample.echo injected plugin panic");
        }
        if context.action_id.as_ref() != SAMPLE_ACTION {
            return Err(KernelError::ActionNotDeclared {
                capability_id: context.capability_id,
                action_id: context.action_id,
            });
        }
        let object = input.0.as_object().ok_or_else(|| {
            KernelError::CapabilityExecution {
                reason: "sample.echo input must be an object".to_owned(),
            }
        })?;
        if object.len() != 1 {
            return Err(KernelError::CapabilityExecution {
                reason: "sample.echo input accepts only message".to_owned(),
            });
        }
        let message = object
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: "sample.echo message must be a string".to_owned(),
            })?;
        let format_version = VersionString::from(VERSION);

        let auxiliary_scope =
            ScopeKey::from(format!("{}:aux", context.state_scope_key.as_ref()));
        let auxiliary_key = StateKey::from("scratch");
        let set = context
            .state
            .set(
                &auxiliary_scope,
                &auxiliary_key,
                &format_version,
                StrictJsonValue(json!({"message": message})),
            )
            .await
            .map_err(state_error)?;
        let conflict = context
            .state
            .compare_and_swap(
                &auxiliary_scope,
                &auxiliary_key,
                set.revision.saturating_sub(1),
                &format_version,
                Some(StrictJsonValue(json!({"message": "must-not-win"}))),
            )
            .await
            .map_err(state_error)?;
        let cas_conflict =
            matches!(conflict, PluginStateCompareAndSwapOutcome::Conflict { .. });
        if !cas_conflict
            || context
                .state
                .get(&auxiliary_scope, &auxiliary_key)
                .await
                .map_err(state_error)?
                .is_none()
        {
            return Err(KernelError::CapabilityExecution {
                reason: "sample.echo CAS conflict was not non-destructive".to_owned(),
            });
        }
        let deleted = context
            .state
            .delete(&auxiliary_scope, &auxiliary_key)
            .await
            .map_err(state_error)?
            .deleted;

        let count_key = StateKey::from("invoke-count");
        for _ in 0..8 {
            let current = context
                .state
                .get(&context.state_scope_key, &count_key)
                .await
                .map_err(state_error)?;
            let expected_revision = current.as_ref().map(|entry| entry.revision).unwrap_or(0);
            let count = current
                .as_ref()
                .and_then(|entry| entry.value.0.as_u64())
                .unwrap_or(0)
                + 1;
            match context
                .state
                .compare_and_swap(
                    &context.state_scope_key,
                    &count_key,
                    expected_revision,
                    &format_version,
                    Some(StrictJsonValue(json!(count))),
                )
                .await
                .map_err(state_error)?
            {
                PluginStateCompareAndSwapOutcome::Applied { .. } => {
                    self.control
                        .successful_effects
                        .fetch_add(1, Ordering::AcqRel);
                    return Ok(StrictJsonValue(json!({
                        "echo": format!("{}{}", self.prefix, message),
                        "count": count,
                        "cas_conflict": cas_conflict,
                        "aux_deleted": deleted
                    })));
                }
                PluginStateCompareAndSwapOutcome::Conflict { .. } => continue,
            }
        }
        Err(KernelError::CapabilityExecution {
            reason: "sample.echo state remained contended".to_owned(),
        })
    }
}

fn state_error(error: PluginStateError) -> KernelError {
    KernelError::CapabilityExecution {
        reason: error.to_string(),
    }
}

fn sample_registration(
    prefix: &str,
    control: Arc<EchoControl>,
) -> Result<PluginRegistration, SampleEchoGateError> {
    let package = package_ref();
    let capability = capability_ref();
    let config_schema = StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "prefix": {"type": "string", "maxLength": 32}
        },
        "required": ["prefix"]
    }));
    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {"type": "string", "maxLength": 256}
        },
        "required": ["message"]
    });
    let output_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "echo": {"type": "string"},
            "count": {"type": "integer", "minimum": 1},
            "cas_conflict": {"type": "boolean"},
            "aux_deleted": {"type": "boolean"}
        },
        "required": ["echo", "count", "cas_conflict", "aux_deleted"]
    });
    let input_digest = digest_payload(&input_schema).map_err(digest_error)?;
    let output_digest = digest_payload(&output_schema).map_err(digest_error)?;
    let capability_manifest = CapabilityManifest {
        id: capability.id.clone(),
        version: capability.version.clone(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display(
            "Sample Echo",
            "Echo one message through the generic capability host.",
        ),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: BTreeSet::from(["desktop".to_owned()]),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({
            "type": "object",
            "additionalProperties": false
        })),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(SAMPLE_ACTION),
                input_schema: CanonicalSchemaRef::from(format!(
                    "schema://{SAMPLE_CAPABILITY}/input@1#{}",
                    input_digest.as_ref()
                )),
                output_schema: CanonicalSchemaRef::from(format!(
                    "schema://{SAMPLE_CAPABILITY}/output@1#{}",
                    output_digest.as_ref()
                )),
                effect_class: EffectClass::WriteReversible,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            context_schema_refs: Vec::new(),
            event_schema_refs: Vec::new(),
            resource_kinds: BTreeSet::from([ResourceKind::from(SAMPLE_RESOURCE_KIND)]),
            host_ports: Vec::new(),
        },
    };
    let skill = SkillDefinition {
        id: SkillId::from(SAMPLE_SKILL),
        version: VersionString::from(VERSION),
        package: package.clone(),
        display: display(
            "Sample Echo Guidance",
            "Use sample.echo to return the exact requested message.",
        ),
        body_ref: LogicalArtifactRef {
            artifact_id: ArtifactId::from("sample.echo-guidance.body"),
            normalized_relative_path: "skills/sample-echo/SKILL.md".to_owned(),
            digest: digest_bytes(b"Use sample.echo with one message."),
        },
        resources: Vec::new(),
        requires_capabilities: vec![capability.clone()],
        supported_surfaces: BTreeSet::from(["desktop".to_owned()]),
    };
    let mcp = McpToolCapabilityMapping {
        package: package.clone(),
        server_id: McpServerId::from(SAMPLE_SERVER),
        canonical_tool_key: McpToolKey::from("sample.echo.server.echo"),
        schema_digest: input_digest,
        capability: capability.clone(),
        materialization_version: VersionString::from(VERSION),
    };
    let manifest = PackageManifest {
        schema_version: VersionString::from(VERSION),
        host_contract_version: VersionString::from(VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display("Sample Echo Package", "CI-only source-neutral fixture."),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: "sample.echo.entrypoint".to_owned(),
            contract_version: VersionString::from(VERSION),
        },
        contributions: PackageContributions {
            capabilities: vec![capability_manifest],
            skills: vec![skill],
            mcp_tools: vec![mcp],
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
        },
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::TestFixture,
        source_identity: SAMPLE_PACKAGE.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: PluginMountId::from(SAMPLE_MOUNT),
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(digest_error)?,
        mount_id: identity.mount_id.clone(),
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
                PluginRegistrarOperation::ContributeSkill,
                PluginRegistrarOperation::ContributeMcpToolMapping,
                PluginRegistrarOperation::BindHostPort,
            ]),
            declared_capability_ids: BTreeSet::from([capability.id.clone()]),
            declared_skill_ids: BTreeSet::from([SkillId::from(SAMPLE_SKILL)]),
            declared_mcp_tool_keys: BTreeSet::from([McpToolKey::from(
                "sample.echo.server.echo",
            )]),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: BTreeSet::from([
                cancellation_port.id.clone(),
                task_port.id.clone(),
            ]),
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).map_err(digest_error)?,
                config_revision: 1,
                value: StrictJsonValue(json!({"prefix": prefix})),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: PluginMountId::from(SAMPLE_MOUNT),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: Vec::new(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from("mount:sample-echo"),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from("mount:sample-echo"),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    registration.add_capability_handler(
        CapabilityId::from(SAMPLE_CAPABILITY),
        Arc::new(EchoHandler {
            prefix: prefix.to_owned(),
            control,
        }),
    )?;
    Ok(registration)
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn package_ref() -> PackageRef {
    PackageRef {
        id: PackageId::from(SAMPLE_PACKAGE),
        version: VersionString::from(VERSION),
    }
}

fn capability_ref() -> CapabilityRef {
    CapabilityRef {
        id: CapabilityId::from(SAMPLE_CAPABILITY),
        version: VersionString::from(VERSION),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(VERSION),
    }
}

fn digest_error(error: impl std::fmt::Display) -> SampleEchoGateError {
    SampleEchoGateError::Digest(error.to_string())
}

fn owner(user_id: &UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: user_id.as_ref().to_owned(),
    }
}

fn resource_binding_dto(owner_id: &str) -> TypedResourceBindingDto {
    TypedResourceBindingDto {
        binding_id: SAMPLE_RESOURCE_BINDING.to_owned(),
        resource_kind: SAMPLE_RESOURCE_KIND.to_owned(),
        resource_id: "echo-target-1".to_owned(),
        owner_id: owner_id.to_owned(),
        operations: BTreeSet::from(["invoke".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
}

fn sample_document(owner_id: &str, instructions: &str) -> AgentPresetDocumentDto {
    AgentPresetDocumentDto {
        schema_version: VERSION.to_owned(),
        surfaces: BTreeSet::from(["desktop".to_owned()]),
        model_route_refs: BTreeMap::from([(
            "agent_chat".to_owned(),
            SAMPLE_MODEL_ROUTE.to_owned(),
        )]),
        chat_route_records: BTreeMap::from([(
            "agent_chat".to_owned(),
            json!({
                "schema": "nomifun.chat-route-record.v1",
                "task": "agent_chat",
                "primary": {
                    "model_route_id": SAMPLE_MODEL_ROUTE,
                    "model_route_revision": 1,
                    "provider_id": "sample-echo-provider",
                    "model": "sample-echo-recorded-model",
                    "protocol": "openai_responses",
                    "connection_config_ref": "sample-echo-connection",
                    "config_revision_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "credential_ref": "sample-echo-credential-ref",
                    "features": [
                        "text_input",
                        "text_output",
                        "tool_calls",
                        "reasoning",
                        "image_input",
                        "audio_input",
                        "audio_output",
                        "prompt_cache",
                        "structured_output",
                        "provider_round_state",
                        "native_responses_items"
                    ]
                },
                "failovers": []
            }),
        )]),
        initial_capabilities: vec![CapabilitySelectionDto {
            capability: ExactCatalogRefDto {
                id: SAMPLE_CAPABILITY.to_owned(),
                version: VERSION.to_owned(),
            },
            required: true,
            exposure: CapabilityExposureDto::Advertised,
            action_allowlist: BTreeSet::from([SAMPLE_ACTION.to_owned()]),
            resource_binding_refs: vec![SAMPLE_RESOURCE_BINDING.to_owned()],
            destination_constraints: BTreeSet::new(),
            context_budget_override: None,
            tool_budget_override: None,
            config: json!({}),
        }],
        on_demand_capabilities: Vec::new(),
        skill_bindings: vec![ExactCatalogRefDto {
            id: SAMPLE_SKILL.to_owned(),
            version: VERSION.to_owned(),
        }],
        resource_bindings: vec![resource_binding_dto(owner_id)],
        system_role_provider_overrides: BTreeMap::new(),
        persona: "Echo fixture".to_owned(),
        instructions: instructions.to_owned(),
        context_policy: json!({}),
        execution_constraints: json!({}),
        runtime_budget: json!({}),
    }
}

fn catalog_from_registry(registry: &MaterializedRegistry) -> CatalogSnapshot {
    CatalogSnapshot {
        capabilities: registry
            .capabilities
            .values()
            .map(|capability| capability.manifest.clone())
            .collect(),
        skills: registry
            .skills
            .values()
            .map(|skill| skill.definition.clone())
            .collect(),
        mcp_tools: registry
            .mcp_tools
            .values()
            .map(|mcp| mcp.mapping.clone())
            .collect(),
        package_sources: registry
            .packages
            .values()
            .map(|package| {
                (
                    PackageRef {
                        id: package.manifest.package_id.clone(),
                        version: package.manifest.package_version.clone(),
                    },
                    package.source.source_kind,
                )
            })
            .collect(),
        unavailable_capabilities: BTreeMap::new(),
        service_key_diagnostics: Vec::new(),
    }
}

fn compiler_release_inputs(
    registry: &MaterializedRegistry,
) -> Result<CompilerReleaseInputs, SampleEchoGateError> {
    Ok(CompilerReleaseInputs {
        resolver_version: VersionString::from(VERSION),
        runtime_protocol_version: VersionString::from(VERSION),
        runtime_feature_inventory_digest: digest_payload(&BTreeSet::<RuntimeFeatureId>::new())
            .map_err(digest_error)?,
        canonical_schema_manifest_digest: canonical_schema_manifest_digest()?,
        target_contribution_manifest_digest: registry.registry_digest.clone(),
        availability_evidence_revision: BUILD_IDENTITY.to_owned(),
    })
}

struct RevisionSet {
    store: Arc<InMemoryControlPlaneStore>,
    control_plane: Arc<AgentControlPlane>,
    clean_revision: nomifun_agent_contracts::AgentPresetRevision,
    clean_snapshot: ResolvedSnapshotEnvelope,
    dirty_revision: nomifun_agent_contracts::AgentPresetRevision,
    dirty_snapshot: ResolvedSnapshotEnvelope,
    clean_action: EditorRevisionActionDto,
    dirty_action: EditorRevisionActionDto,
    save_fault_rejected: bool,
}

async fn build_revisions(
    registry: &MaterializedRegistry,
    owner_id: &UserId,
) -> Result<RevisionSet, SampleEchoGateError> {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let catalog_snapshot = catalog_from_registry(registry);
    let catalog = Arc::new(StaticCatalogProvider::new(catalog_snapshot.clone()));
    let templates = OfficialTemplateCatalog::load()?;
    let release = compiler_release_inputs(registry)?;
    let compiler = PresetPreviewCompiler::new(release.clone(), templates.clone())
        .with_materialized_registry(
            Arc::new(registry.clone()),
            CompilerEnvironment {
                resolver_version: release.resolver_version.clone(),
                required_runtime_protocol_version: release.runtime_protocol_version.clone(),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: release
                    .runtime_feature_inventory_digest
                    .clone(),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: release
                    .canonical_schema_manifest_digest
                    .clone(),
                target_contribution_manifest_digest: release
                    .target_contribution_manifest_digest
                    .clone(),
                host_target: RuntimeTarget::from(native_target_id()),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: BUILD_IDENTITY.to_owned(),
            },
        );
    let control_plane = Arc::new(AgentControlPlane::new(
        store.clone(),
        catalog,
        templates,
        compiler.clone(),
    ));
    let created = control_plane
        .create_preset(
            owner_id,
            CreateAgentPresetRequest {
                display_name: "Sample Echo".to_owned(),
                description: Some("C6 compiled plugin gate".to_owned()),
                fork_from_revision: None,
            },
        )
        .await?;
    let preset_id = created.preset.preset_id.clone();
    let mut initial_draft = created.draft;
    initial_draft.document =
        sample_document(owner_id.as_ref(), "Use sample.echo for the requested value.");
    let initial_request = preview_request(initial_draft.clone());
    let initial_compilation = compiler.compile(
        owner_id,
        &initial_request,
        None,
        None,
        None,
        &catalog_snapshot,
    )?;
    invariant(
        initial_compilation.response.can_create_session,
        "initial Preview was blocked",
    )?;
    let initial_plan = control_plane.build_editor_test_plan(
        EditorDraftStateDto::Dirty,
        initial_compilation.response.clone(),
        initial_draft.clone(),
        Some("C6 initial sample.echo Revision".to_owned()),
    )?;
    let _initial_save = initial_plan.save_request.clone().ok_or_else(|| {
        SampleEchoGateError::Invariant("dirty Test plan omitted ordinary save request".to_owned())
    })?;
    let initial_snapshot = initial_compilation.snapshot.clone().ok_or_else(|| {
        SampleEchoGateError::Invariant("initial Preview omitted Snapshot".to_owned())
    })?;
    let initial_revision = nomifun_agent_contracts::AgentPresetRevision {
        reference: initial_compilation.candidate_revision_ref,
        payload: initial_compilation.payload,
        created_by: owner_id.clone(),
        created_at_ms: initial_snapshot.created_at_ms,
        reason: Some("C6 initial sample.echo Revision".to_owned()),
    };
    initial_revision
        .validate()
        .map_err(|error| SampleEchoGateError::Invariant(error.message))?;
    store
        .append_revision(
            None,
            initial_revision.clone(),
            initial_snapshot.clone(),
            initial_draft.display_name,
            initial_draft.description,
        )
        .await?;

    let clean_editor = control_plane.editor(owner_id, &preset_id, None).await?;
    let clean_preview = control_plane
        .preview(
            owner_id,
            &preset_id,
            preview_request(clean_editor.draft.clone()),
        )
        .await?;
    let clean_plan = control_plane.build_editor_test_plan(
        EditorDraftStateDto::Clean,
        clean_preview,
        clean_editor.draft.clone(),
        None,
    )?;
    invariant(clean_plan.save_request.is_none(), "clean Test attempted a save")?;

    let mut dirty_draft = clean_editor.draft;
    dirty_draft.document.instructions =
        "Use sample.echo and preserve the returned receipt.".to_owned();
    let dirty_request = preview_request(dirty_draft.clone());
    let dirty_compilation = compiler.compile(
        owner_id,
        &dirty_request,
        Some(&initial_revision),
        Some(&initial_snapshot),
        None,
        &catalog_snapshot,
    )?;
    let dirty_plan = control_plane.build_editor_test_plan(
        EditorDraftStateDto::Dirty,
        dirty_compilation.response.clone(),
        dirty_draft.clone(),
        Some("C6 dirty sample.echo Revision".to_owned()),
    )?;
    let valid_save = dirty_plan.save_request.clone().ok_or_else(|| {
        SampleEchoGateError::Invariant("dirty Test plan omitted save request".to_owned())
    })?;
    let mut stale_save = valid_save.clone();
    stale_save.preview_digest = "stale-preview-digest".to_owned();
    let save_fault_rejected = control_plane
        .save_revision(owner_id, &preset_id, stale_save)
        .await
        .is_err();
    invariant(save_fault_rejected, "stale Preview save unexpectedly succeeded")?;
    invariant(
        store
            .get_revision_number(&AgentPresetId::from(preset_id.clone()), 2)
            .await?
            .is_none(),
        "failed save published Revision 2",
    )?;
    let dirty_snapshot = dirty_compilation.snapshot.clone().ok_or_else(|| {
        SampleEchoGateError::Invariant("dirty Preview omitted Snapshot".to_owned())
    })?;
    let dirty_revision = nomifun_agent_contracts::AgentPresetRevision {
        reference: dirty_compilation.candidate_revision_ref,
        payload: dirty_compilation.payload,
        created_by: owner_id.clone(),
        created_at_ms: dirty_snapshot.created_at_ms,
        reason: valid_save.reason,
    };
    dirty_revision
        .validate()
        .map_err(|error| SampleEchoGateError::Invariant(error.message))?;
    store
        .append_revision(
            Some(&initial_revision.reference),
            dirty_revision.clone(),
            dirty_snapshot.clone(),
            dirty_draft.display_name,
            dirty_draft.description,
        )
        .await?;

    let clean_ref = initial_revision.reference.clone();
    let dirty_ref = dirty_revision.reference.clone();
    let clean_revision = store
        .get_revision(&clean_ref)
        .await?
        .ok_or_else(|| SampleEchoGateError::Invariant("clean Revision missing".to_owned()))?;
    let clean_snapshot = store
        .get_snapshot(&clean_ref)
        .await?
        .ok_or_else(|| SampleEchoGateError::Invariant("clean Snapshot missing".to_owned()))?;
    let dirty_revision = store
        .get_revision(&dirty_ref)
        .await?
        .ok_or_else(|| SampleEchoGateError::Invariant("dirty Revision missing".to_owned()))?;
    let dirty_snapshot = store
        .get_snapshot(&dirty_ref)
        .await?
        .ok_or_else(|| SampleEchoGateError::Invariant("dirty Snapshot missing".to_owned()))?;
    Ok(RevisionSet {
        store,
        control_plane,
        clean_revision,
        clean_snapshot,
        dirty_revision,
        dirty_snapshot,
        clean_action: clean_plan.revision_action,
        dirty_action: dirty_plan.revision_action,
        save_fault_rejected,
    })
}

fn preview_request(draft: AgentPresetDraftDto) -> ResolveAgentPresetPreviewRequest {
    ResolveAgentPresetPreviewRequest {
        expected_current_revision: draft.current_revision.clone(),
        draft,
        scene: "agent_settings".to_owned(),
        surface: "desktop".to_owned(),
        audience: "owner".to_owned(),
    }
}

fn compile_kernel_snapshot(
    registry: &MaterializedRegistry,
    revision: nomifun_agent_contracts::AgentPresetRevision,
    owner_ref: PrincipalRef,
) -> Result<CompiledSnapshot, SampleEchoGateError> {
    Ok(AgentPresetCompiler::compile(
        registry,
        &CompilerEnvironment {
            resolver_version: VersionString::from(VERSION),
            required_runtime_protocol_version: VersionString::from(VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: digest_payload(
                &BTreeSet::<RuntimeFeatureId>::new(),
            )
            .map_err(digest_error)?,
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: canonical_schema_manifest_digest()?,
            target_contribution_manifest_digest: registry.registry_digest.clone(),
            host_target: RuntimeTarget::from(native_target_id()),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: BUILD_IDENTITY.to_owned(),
        },
        CompileRequest {
            revision,
            principal: owner_ref,
            scene: "agent_settings".to_owned(),
            surface: "desktop".to_owned(),
            audience: "owner".to_owned(),
            created_at_ms: now_ms(),
            resolver_run_id: OperationId::from(new_id("resolver")),
        },
    )?)
}

fn bind_canonical_snapshot(
    mut compiled: CompiledSnapshot,
    canonical: &ResolvedSnapshotEnvelope,
) -> Result<CompiledSnapshot, SampleEchoGateError> {
    let compiled_initial = compiled
        .content()
        .initial_capabilities
        .iter()
        .map(|capability| capability.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let canonical_initial = canonical
        .content
        .initial_capabilities
        .iter()
        .map(|capability| capability.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let compiled_on_demand = compiled
        .content()
        .on_demand_capabilities
        .iter()
        .map(|capability| capability.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let canonical_on_demand = canonical
        .content
        .on_demand_capabilities
        .iter()
        .map(|capability| capability.capability.id.clone())
        .collect::<BTreeSet<_>>();
    invariant(
        compiled_initial == canonical_initial
            && compiled_on_demand == canonical_on_demand
            && compiled.content().capability_allowlist == canonical.content.capability_allowlist
            && compiled.content().typed_resource_bindings
                == canonical.content.typed_resource_bindings
            && compiled.content().skill_locks == canonical.content.skill_locks
            && compiled.content().mcp_tool_locks == canonical.content.mcp_tool_locks,
        "Control Plane and Kernel execution ceilings differ",
    )?;
    compiled.envelope = canonical.clone();
    Ok(compiled)
}

#[derive(Clone)]
struct RecordedProviderTransport {
    scripts: Arc<Mutex<VecDeque<Vec<ProviderWireFrame>>>>,
    calls: Arc<AtomicUsize>,
}

impl RecordedProviderTransport {
    fn new(scripts: Vec<Vec<ProviderWireFrame>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Arc::new(Mutex::new(scripts.into())),
            calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ProviderTransport for RecordedProviderTransport {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        if credential.credential_ref() != &request.credential_ref
            || credential.target().model_route_id != request.route_identity.route_id
            || credential.target().model_route_revision != request.route_identity.route_revision
        {
            return Err(ChatModelError::protocol_violation(
                "recorded credential target mismatch",
            ));
        }
        self.calls.fetch_add(1, Ordering::AcqRel);
        let frames = self
            .scripts
            .lock()
            .map_err(|_| ChatModelError::provider_unavailable("recorded transport lock"))?
            .pop_front()
            .ok_or_else(|| ChatModelError::provider_unavailable("recorded transport exhausted"))?;
        Ok(Box::pin(stream::iter(frames.into_iter().map(Ok))))
    }
}

struct ExactCausalityGate {
    session_id: AgentSessionId,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
}

#[async_trait]
impl ChatCausalityGate for ExactCausalityGate {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        if causality.agent_session_id != self.session_id
            || causality.resolved_snapshot_ref != self.snapshot
        {
            return Err(ChatModelError::protocol_violation(
                "recorded causality is outside the AgentSession",
            ));
        }
        Ok(())
    }
}

struct ExactRouteResolver {
    route: ResolvedChatRoute,
}

#[async_trait]
impl ChatRouteResolver for ExactRouteResolver {
    async fn resolve(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatModelError> {
        if selection.route_id != self.route.model_route_id
            || selection.route_revision != self.route.model_route_revision
        {
            return Err(ChatModelError::protocol_violation(
                "recorded route selection mismatch",
            ));
        }
        Ok(ResolvedChatRouteSet {
            primary: self.route.clone(),
            failovers: Vec::new(),
        })
    }
}

struct ExactCredentialStore;

#[async_trait]
impl ProviderCredentialStore for ExactCredentialStore {
    async fn lease(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
    ) -> Result<CredentialLease, ChatModelError> {
        Ok(CredentialLease::new(
            credential_ref.clone(),
            target.clone(),
            "sample-echo-recorded-handle",
        ))
    }
}

fn recorded_broker(
    session_id: AgentSessionId,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
) -> Result<(Arc<ChatModelBroker>, Arc<RecordedProviderTransport>), SampleEchoGateError> {
    let transport = RecordedProviderTransport::new(vec![
        vec![
            wire_frame(
                "response.created",
                json!({"response": {"id": "sample-echo-tool-round"}}),
            ),
            wire_frame(
                "response.function_call.done",
                json!({
                    "call_id": "sample-echo-call-1",
                    "name": SAMPLE_CAPABILITY,
                    "arguments": {"message": "hello"}
                }),
            ),
            wire_frame(
                "response.completed",
                json!({"response": {"status": "completed"}, "finish_reason": "tool_calls"}),
            ),
        ],
        vec![
            wire_frame(
                "response.created",
                json!({"response": {"id": "sample-echo-final-round"}}),
            ),
            wire_frame(
                "response.output_text.delta",
                json!({"delta": "echo:hello"}),
            ),
            wire_frame(
                "response.completed",
                json!({"response": {"status": "completed"}, "finish_reason": "stop"}),
            ),
        ],
    ]);
    let provider_transport: Arc<dyn ProviderTransport> = transport.clone();
    let adapters: Vec<Arc<dyn ChatProtocolAdapter>> = vec![
        Arc::new(AnthropicAdapter::new(provider_transport.clone())),
        Arc::new(OpenAiChatAdapter::new(provider_transport.clone())),
        Arc::new(OpenAiResponsesAdapter::new(provider_transport.clone())),
        Arc::new(GeminiAdapter::new(provider_transport.clone())),
        Arc::new(BedrockAdapter::new(provider_transport.clone())),
        Arc::new(VertexAdapter::new(provider_transport)),
    ];
    let route = recorded_route();
    let broker = ChatModelBroker::new(
        Arc::new(ExactCausalityGate {
            session_id,
            snapshot,
        }),
        Arc::new(ExactRouteResolver { route }),
        Arc::new(ExactCredentialStore),
        adapters,
        BrokerRetryPolicy {
            max_total_attempts: 1,
            max_attempts_per_route: 1,
        },
    )?;
    Ok((Arc::new(broker), transport))
}

fn recorded_route() -> ResolvedChatRoute {
    ResolvedChatRoute {
        model_route_id: ModelRouteId::from(SAMPLE_MODEL_ROUTE),
        model_route_revision: 1,
        provider_id: ProviderIdRef("sample-echo-provider".to_owned()),
        model: "sample-echo-recorded-model".to_owned(),
        protocol: ChatProtocol::OpenaiResponses,
        connection_config_ref: ConnectionConfigRef::from("sample-echo-connection"),
        config_revision_digest: DigestHex::from("a".repeat(64)),
        credential_ref: ProviderCredentialRef("sample-echo-credential-ref".to_owned()),
        features: protocol_features(ChatProtocol::OpenaiResponses),
    }
}

fn wire_frame(event: &str, data: Value) -> ProviderWireFrame {
    ProviderWireFrame {
        event: event.to_owned(),
        data,
    }
}

fn model_request(
    session_id: &AgentSessionId,
    snapshot: &nomifun_agent_contracts::ResolvedSnapshotRef,
    turn: &OperationId,
    cause: &EventId,
    messages: Vec<ChatMessage>,
) -> ChatModelRequest {
    let route_identity = ChatRouteIdentity::new(
        "sample.echo@1",
        nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
        SAMPLE_MODEL_ROUTE.into(),
        1,
    );
    ChatModelRequest {
        contract_version: VersionString::from("chat-model-v1"),
        causality: ChatCausality {
            agent_session_id: session_id.clone(),
            turn_operation_id: turn.clone(),
            causation_event_id: cause.clone(),
            resolved_snapshot_ref: snapshot.clone(),
            route_identity: route_identity.clone(),
            operation_id: OperationId::from(new_id("model")),
        },
        route: route_identity,
        input: ChatModelInput {
            instructions: vec!["Use sample.echo exactly once.".to_owned()],
            messages,
            tools: vec![ChatToolDefinition {
                name: SAMPLE_CAPABILITY.to_owned(),
                description: "Echo one message.".to_owned(),
                input_schema: StrictJsonValue(json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {"message": {"type": "string"}},
                    "required": ["message"]
                })),
                deferred: false,
            }],
            tool_choice: ChatToolChoice::Auto,
            max_output_tokens: Some(128),
            reasoning: None,
            prompt_cache: PromptCachePolicy::Disabled,
            response_format: ChatResponseFormat::Text,
            requested_output_modalities: BTreeSet::from([ChatModality::Text]),
            provider_round_parent: None,
            preserve_native_responses_items: false,
            metadata: BTreeMap::from([("fixture".to_owned(), SAMPLE_PACKAGE.to_owned())]),
        },
    }
}

#[derive(Clone)]
struct CommittedEffect {
    start: NativeActionStart,
    started_event_id: EventId,
}

struct SampleEchoIngress {
    store: Arc<AgentSessionStore>,
    session_id: AgentSessionId,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pending_tools: AsyncMutex<BTreeMap<String, EventId>>,
    effects: AsyncMutex<BTreeMap<String, CommittedEffect>>,
}

impl SampleEchoIngress {
    fn new(
        store: Arc<AgentSessionStore>,
        session_id: AgentSessionId,
        snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            session_id,
            snapshot,
            pending_tools: AsyncMutex::new(BTreeMap::new()),
            effects: AsyncMutex::new(BTreeMap::new()),
        })
    }

    async fn register_tool(&self, operation: &OperationId, event_id: EventId) {
        self.pending_tools
            .lock()
            .await
            .insert(operation.as_ref().to_owned(), event_id);
    }

    async fn committed_effect(
        &self,
        operation: &OperationId,
    ) -> Result<CommittedEffect, SampleEchoGateError> {
        self.effects
            .lock()
            .await
            .get(operation.as_ref())
            .cloned()
            .ok_or_else(|| {
                SampleEchoGateError::Invariant(
                    "runtime action returned without durable effect/started".to_owned(),
                )
            })
    }
}

#[async_trait]
impl RuntimeIngressPort for SampleEchoIngress {
    async fn append_runtime_event(
        &self,
        event: RuntimeEventEnvelope,
    ) -> Result<RuntimeEventAck, RuntimeError> {
        self.store
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id: self.session_id.clone(),
                envelope: event,
            })
            .await
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
            .ack
            .ok_or_else(|| RuntimeError::Protocol("runtime event was not persisted".to_owned()))
    }

    async fn commit_native_action_start(
        &self,
        start: NativeActionStart,
    ) -> Result<NativeActionStartAck, RuntimeError> {
        if start.agent_session_id != self.session_id
            || start.snapshot_digest != self.snapshot.snapshot_digest
            || start.capability_id.as_ref() != SAMPLE_CAPABILITY
            || start.action_id.as_ref() != SAMPLE_ACTION
            || start.active_set_generation != 0
            || start.resource_binding_ids
                != BTreeSet::from([ResourceBindingId::from(SAMPLE_RESOURCE_BINDING)])
        {
            return Err(RuntimeError::Protocol(
                "sample.echo native action escaped its Snapshot or resource binding".to_owned(),
            ));
        }
        let tool_event = self
            .pending_tools
            .lock()
            .await
            .get(start.turn_operation_id.as_ref())
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol("native action has no tool event".to_owned()))?;
        let event_id = EventId::from(new_id("effect-started"));
        let result = self
            .store
            .record_effect_started(EffectEventRequest {
                agent_session_id: self.session_id.clone(),
                event_id: event_id.clone(),
                producer_id: EventProducerId::from("capability-host"),
                idempotency_key: start.idempotency_key.clone(),
                correlation_id: CorrelationId::from(start.effect_id.as_ref().to_owned()),
                strategy: EffectStrategy::ManagedEffect,
                causation_event_id: Some(tool_event),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "effect_id": start.effect_id,
                    "capability_id": start.capability_id,
                    "action_id": start.action_id,
                    "resource_binding_ids": start.resource_binding_ids
                }))),
            })
            .await
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
        let ack = result.ack.ok_or_else(|| {
            RuntimeError::Protocol("effect/started did not commit an ACK".to_owned())
        })?;
        self.effects.lock().await.insert(
            start.turn_operation_id.as_ref().to_owned(),
            CommittedEffect {
                start: start.clone(),
                started_event_id: event_id.clone(),
            },
        );
        Ok(NativeActionStartAck {
            agent_session_id: start.agent_session_id,
            runtime_binding_id: start.runtime_binding_id,
            turn_operation_id: start.turn_operation_id,
            action_id: start.action_id,
            effect_id: start.effect_id,
            idempotency_key: start.idempotency_key,
            capability_id: start.capability_id,
            active_set_generation: start.active_set_generation,
            snapshot_digest: start.snapshot_digest,
            effect_started_event_id: event_id,
            committed_session_seq: ack.seq,
        })
    }
}

struct RuntimeFixture {
    release: RuntimeReleaseDescriptor,
    hello: RuntimeHelloPayload,
    build_digest: DigestHex,
    executable_digest: DigestHex,
    target_id: String,
}

fn prepare_runtime_fixture(
    directory: &Path,
    node_executable: &Path,
    compiled: &CompiledSnapshot,
    binding: &RuntimeBindingContract,
    opening_event_id: &EventId,
    dispose_mode: &str,
) -> Result<RuntimeFixture, SampleEchoGateError> {
    std::fs::create_dir_all(directory)?;
    std::fs::write(directory.join(RUNTIME_SCRIPT_FILE), RECORDED_RUNTIME_SCRIPT)?;
    let node_digest = sha256_path(node_executable)?;
    let script_digest = digest_bytes(RECORDED_RUNTIME_SCRIPT.as_bytes());
    let build_digest = digest_payload(&(node_digest.clone(), script_digest.clone()))
        .map_err(digest_error)?;
    let target_id = native_target_id().to_owned();
    let release = RuntimeReleaseDescriptor::pinned_contract()?;
    let runtime_target = release.runtime_target_for_target(&target_id)?;
    let native_features = compiled.content().required_runtime_features.clone();
    let native_actions = BTreeSet::from([ActionId::from(SAMPLE_ACTION)]);
    let expectation = release.hello_expectation(
        build_digest.clone(),
        runtime_target,
        native_features,
        native_actions,
    );
    let hello = RuntimeHelloPayload {
        runtime_release_digest: expectation.runtime_release_digest,
        runtime_build_digest: expectation.runtime_build_digest,
        fork_commit: expectation.fork_commit,
        tracked_upstream_commit: expectation.tracked_upstream_commit,
        protocol_version: expectation.protocol_version,
        protocol_schema_digest: expectation.protocol_schema_digest,
        runtime_target: expectation.runtime_target,
        supported_profiles: expectation.supported_profiles,
        native_features: expectation.native_features,
        native_actions: expectation.native_actions,
        full_auto: expectation.full_auto,
        rpc_allowlist: expectation.rpc_allowlist,
    };
    let runtime_bound_event = RuntimeEventEnvelope {
        runtime_binding_id: binding.runtime_binding_id.clone(),
        producer_seq: 1,
        event_id: binding.runtime_bound_event_id.clone(),
        idempotency_key: IdempotencyKey::from(format!(
            "runtime-bound:{}",
            binding.runtime_binding_id.as_ref()
        )),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("runtime/bound".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId::from(binding.runtime_binding_id.as_ref().to_owned()),
            causation_event_id: Some(opening_event_id.clone()),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "runtime_release_digest": binding.runtime_release_digest,
                "runtime_build_digest": binding.runtime_build_digest,
                "protocol_version": binding.protocol_version,
                "runtime_profile_digest": binding.runtime_profile_digest,
                "snapshot_digest": binding.resolved_snapshot_ref.snapshot_digest,
                "through_seq": binding.through_seq
            }))),
        },
    };
    std::fs::write(
        directory.join(RUNTIME_CONFIG_FILE),
        serde_json::to_vec(&json!({
            "hello": hello,
            "binding": binding,
            "runtime_bound_event": runtime_bound_event,
            "action": {
                "action_id": SAMPLE_ACTION,
                "capability_id": SAMPLE_CAPABILITY,
                "resource_binding_ids": [SAMPLE_RESOURCE_BINDING]
            },
            "dispose_mode": dispose_mode
        }))?,
    )?;
    Ok(RuntimeFixture {
        release,
        hello,
        build_digest,
        executable_digest: node_digest,
        target_id,
    })
}

fn sha256_path(path: &Path) -> Result<DigestHex, SampleEchoGateError> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(DigestHex::from(hex_lower(&digest.finalize())))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn native_target_id() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows_desktop_x64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos_desktop_arm64"
    } else if cfg!(target_os = "macos") {
        "macos_desktop_x64"
    } else {
        "linux_desktop_x64"
    }
}

struct OpenSession {
    session_id: AgentSessionId,
    binding: AgentBindingValue,
    ready_event_id: EventId,
    runtime_binding: RuntimeBindingContract,
    runtime: Arc<ManagedRuntimeSession>,
    ingress: Arc<SampleEchoIngress>,
}

async fn open_persistent_session(
    session_store: Arc<AgentSessionStore>,
    supervisor: Arc<CodexRuntimeSupervisor>,
    registry: &MaterializedRegistry,
    compiled: &CompiledSnapshot,
    revision: &nomifun_agent_contracts::AgentPresetRevision,
    owner_ref: &PrincipalRef,
    node_executable: &Path,
    runtime_directory: &Path,
    dispose_mode: &str,
) -> Result<OpenSession, SampleEchoGateError> {
    let session_id = AgentSessionId::from(Uuid::now_v7().to_string());
    let binding = AgentBindingValue {
        preset_revision_ref: revision.reference.clone(),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        typed_resource_bindings: revision.payload.resource_bindings.clone(),
        binding_version: 1,
    };
    let mut create_request = CreateSessionRequest::new(
        AgentSessionLiveRecord {
            agent_session_id: session_id.clone(),
            owner_ref: owner_ref.clone(),
            metadata: AgentSessionMetadata {
                title: Some("sample.echo C6 gate".to_owned()),
                archived: false,
                pinned: false,
            },
            agent_binding: binding.clone(),
            remote_binding_provenance: None,
            parent_session_id: None,
            fork_base_payload_id: None,
            next_seq: 1,
        },
        now_ms(),
        OperationId::from(new_id("session-open")),
        EventProducerId::from("session-api"),
        IdempotencyKey::from(new_id("session-open")),
        CorrelationId::from(new_id("session")),
    );
    create_request.initial_active_capability_ids = compiled
        .content()
        .initial_capabilities
        .iter()
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect();
    let create = session_store.create_session(create_request).await?;
    let runtime_binding_id = RuntimeBindingId::from(new_id("runtime-binding"));
    let runtime_bound_event_id = EventId::from(new_id("runtime-bound"));
    let placeholder_release = RuntimeReleaseDescriptor::pinned_contract()?;
    let runtime_binding = RuntimeBindingContract {
        runtime_binding_id: runtime_binding_id.clone(),
        agent_session_id: session_id.clone(),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        runtime_release_digest: placeholder_release.contract_digest,
        runtime_build_digest: DigestHex::from("0".repeat(64)),
        protocol_version: VersionString::from(VERSION),
        profile_kind: RuntimeProfileKind::ManagedMinimal,
        runtime_profile_digest: compiled
            .content()
            .compiled_runtime_profile_digest
            .clone(),
        active_set_generation: 0,
        runtime_bound_event_id: runtime_bound_event_id.clone(),
        through_seq: create.activation_ack.seq,
    };
    let fixture = prepare_runtime_fixture(
        runtime_directory,
        node_executable,
        compiled,
        &runtime_binding,
        &create.opening_ack.event_id,
        dispose_mode,
    )?;
    let runtime_binding = RuntimeBindingContract {
        runtime_release_digest: fixture.release.contract_digest.clone(),
        runtime_build_digest: fixture.build_digest.clone(),
        ..runtime_binding
    };
    prepare_runtime_fixture(
        runtime_directory,
        node_executable,
        compiled,
        &runtime_binding,
        &create.opening_ack.event_id,
        dispose_mode,
    )?;
    let profile = PinnedRuntimeProfile {
        kind: RuntimeProfileKind::ManagedMinimal,
        runtime_protocol_version: VersionString::from(VERSION),
        profile_digest: compiled
            .content()
            .compiled_runtime_profile_digest
            .clone(),
        enabled_runtime_features: compiled.content().required_runtime_features.clone(),
        initial_capabilities: compiled
            .content()
            .initial_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect(),
        on_demand_capabilities: compiled
            .content()
            .on_demand_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect(),
        typed_resource_bindings: compiled.content().typed_resource_bindings.clone(),
    };
    let context = RuntimeCommandContext {
        agent_session_id: session_id.clone(),
        runtime_binding_id: runtime_binding_id.clone(),
        operation_id: OperationId::from(new_id("runtime-create")),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        runtime_profile_digest: profile.profile_digest.clone(),
        active_set_generation: 0,
    };
    let ingress = SampleEchoIngress::new(
        session_store.clone(),
        session_id.clone(),
        compiled.snapshot_ref().clone(),
    );
    let process = RuntimeProcessConfig::pinned_app_server(
        node_executable,
        runtime_directory,
        &fixture.target_id,
        fixture.executable_digest.clone(),
        &fixture.release,
    )?;
    let runtime = supervisor
        .launch(RuntimeLaunchRequest {
            process,
            credential: InheritedHandleCredential::new(
                b"sample-echo-runtime-credential".to_vec(),
            )?,
            release: fixture.release.clone(),
            hello_expectation:
                nomifun_codex_runtime::RuntimeHelloExpectation::from_payload(
                    fixture.hello.clone(),
                ),
            profile,
            open_command: RuntimeCommand::Create(RuntimeCreateParams {
                context,
                profile_kind: RuntimeProfileKind::ManagedMinimal,
                full_auto: FullAutoExecutionWire::fixed(),
                initial_capabilities: compiled
                    .content()
                    .initial_capabilities
                    .iter()
                    .map(|capability| capability.capability.id.clone())
                    .collect(),
                on_demand_capabilities: compiled
                    .content()
                    .on_demand_capabilities
                    .iter()
                    .map(|capability| capability.capability.id.clone())
                    .collect(),
                typed_resource_bindings: compiled.content().typed_resource_bindings.clone(),
            }),
            ingress: ingress.clone(),
            client_limits: ClientLimits::default(),
            dispose_timeout: Duration::from_millis(300),
        })
        .await?;
    invariant(
        runtime.binding() == &runtime_binding,
        "Runtime binding ACK differed from the final release binding",
    )?;
    let ready_event_id = EventId::from(new_id("session-ready"));
    append_event(
        &session_store,
        &session_id,
        ready_event_id.clone(),
        "runtime-supervisor",
        new_id("session-ready"),
        "session/ready",
        CorrelationId::from(session_id.as_ref().to_owned()),
        Some(create.opening_ack.event_id.clone()),
        json!({"runtime_binding_id": runtime_binding_id}),
    )
    .await?;
    invariant(
        registry.capabilities.contains_key(&CapabilityId::from(SAMPLE_CAPABILITY)),
        "Session opened without materialized sample.echo",
    )?;
    Ok(OpenSession {
        session_id,
        binding,
        ready_event_id,
        runtime_binding,
        runtime,
        ingress,
    })
}

struct TurnStart {
    operation: OperationId,
    turn_event_id: EventId,
    user_event_id: EventId,
}

async fn begin_turn(
    store: &AgentSessionStore,
    session: &OpenSession,
    message: &str,
) -> Result<TurnStart, SampleEchoGateError> {
    let operation = OperationId::from(new_id("turn"));
    let turn_event_id = EventId::from(new_id("turn-started"));
    append_event(
        store,
        &session.session_id,
        turn_event_id.clone(),
        "runtime-supervisor",
        new_id("turn-started"),
        "turn/started",
        CorrelationId::from(operation.as_ref().to_owned()),
        Some(session.ready_event_id.clone()),
        json!({"operation_id": operation}),
    )
    .await?;
    let user_event_id = EventId::from(new_id("user-message"));
    append_event(
        store,
        &session.session_id,
        user_event_id.clone(),
        "session-api",
        new_id("user-message"),
        "message/user-accepted",
        CorrelationId::from(new_id("user-message")),
        Some(turn_event_id.clone()),
        json!({"content": message}),
    )
    .await?;
    Ok(TurnStart {
        operation,
        turn_event_id,
        user_event_id,
    })
}

enum ToolExecution {
    Succeeded {
        output: StrictJsonValue,
        tool_result_event_id: EventId,
    },
    Panicked,
}

async fn execute_tool(
    store: Arc<AgentSessionStore>,
    registry: Arc<KernelRegistry>,
    compiled: CompiledSnapshot,
    session: &OpenSession,
    turn: &TurnStart,
    call: ChatToolCall,
) -> Result<ToolExecution, SampleEchoGateError> {
    let tool_event_id = EventId::from(new_id("tool-started"));
    append_event(
        &store,
        &session.session_id,
        tool_event_id.clone(),
        "runtime-supervisor",
        new_id("tool-started"),
        "tool/call-started",
        CorrelationId::from(call.call_id.as_ref().to_owned()),
        Some(turn.turn_event_id.clone()),
        json!({"name": call.name, "arguments": call.arguments}),
    )
    .await?;
    session
        .ingress
        .register_tool(&turn.operation, tool_event_id.clone())
        .await;
    let _: Value = session
        .runtime
        .client()
        .command(&RuntimeCommand::StartTurn(RuntimeStartTurnParams {
            context: RuntimeCommandContext {
                agent_session_id: session.session_id.clone(),
                runtime_binding_id: session.runtime_binding.runtime_binding_id.clone(),
                operation_id: turn.operation.clone(),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                runtime_profile_digest: compiled
                    .content()
                    .compiled_runtime_profile_digest
                    .clone(),
                active_set_generation: 0,
            },
            idempotency_key: IdempotencyKey::from(format!(
                "turn:{}",
                turn.operation.as_ref()
            )),
            input_event_id: turn.user_event_id.clone(),
        }))
        .await?;
    let effect = session.ingress.committed_effect(&turn.operation).await?;
    let active = SessionCapabilityState::new(&compiled).snapshot()?;
    let invocation = CapabilityInvocationRequest {
        principal: session.binding_owner(),
        session_owner: session.binding_owner(),
        agent_session_id: session.session_id.clone(),
        operation_id: turn.operation.clone(),
        idempotency_key: effect.start.idempotency_key.clone(),
        correlation_id: CorrelationId::from(effect.start.effect_id.as_ref().to_owned()),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        active_set_generation: 0,
        capability_id: CapabilityId::from(SAMPLE_CAPABILITY),
        action_id: ActionId::from(SAMPLE_ACTION),
        resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
            SAMPLE_RESOURCE_BINDING,
        )]),
        state_scope_key: ScopeKey::from(format!("session:{}", session.session_id.as_ref())),
        input: call.arguments,
    };
    let task = tokio::spawn(async move {
        registry.invoke(&compiled, &active, invocation).await
    });
    match task.await {
        Ok(Ok(output)) => {
            let terminal = EventId::from(new_id("effect-succeeded"));
            store
                .record_effect_terminal(
                    EffectEventRequest {
                        agent_session_id: session.session_id.clone(),
                        event_id: terminal.clone(),
                        producer_id: EventProducerId::from("owning-plugin"),
                        idempotency_key: effect.start.idempotency_key.clone(),
                        correlation_id: CorrelationId::from(
                            effect.start.effect_id.as_ref().to_owned(),
                        ),
                        strategy: EffectStrategy::ManagedEffect,
                        causation_event_id: Some(effect.started_event_id.clone()),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                            "receipt": output
                        }))),
                    },
                    EffectTerminalState::Succeeded,
                )
                .await?;
            let tool_result_event_id = EventId::from(new_id("tool-result"));
            append_event(
                &store,
                &session.session_id,
                tool_result_event_id.clone(),
                "owning-plugin",
                new_id("tool-result"),
                "tool/result-recorded",
                CorrelationId::from(call.call_id.as_ref().to_owned()),
                Some(terminal),
                json!({"output": output}),
            )
            .await?;
            Ok(ToolExecution::Succeeded {
                output,
                tool_result_event_id,
            })
        }
        Ok(Err(error)) => Err(error.into()),
        Err(error) if error.is_panic() => {
            store
                .record_effect_terminal(
                    EffectEventRequest {
                        agent_session_id: session.session_id.clone(),
                        event_id: EventId::from(new_id("effect-uncertain")),
                        producer_id: EventProducerId::from("runtime-supervisor"),
                        idempotency_key: effect.start.idempotency_key.clone(),
                        correlation_id: CorrelationId::from(
                            effect.start.effect_id.as_ref().to_owned(),
                        ),
                        strategy: EffectStrategy::ManagedEffect,
                        causation_event_id: Some(effect.started_event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                            "reason": "plugin_panic"
                        }))),
                    },
                    EffectTerminalState::Uncertain,
                )
                .await?;
            Ok(ToolExecution::Panicked)
        }
        Err(error) => Err(SampleEchoGateError::Join(error.to_string())),
    }
}

impl OpenSession {
    fn binding_owner(&self) -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: self.binding.typed_resource_bindings[0].owner_id.clone(),
        }
    }
}

async fn append_event(
    store: &AgentSessionStore,
    session_id: &AgentSessionId,
    event_id: EventId,
    producer: &str,
    idempotency_key: String,
    kind: &str,
    correlation_id: CorrelationId,
    causation_event_id: Option<EventId>,
    payload: Value,
) -> Result<EventId, SampleEchoGateError> {
    store
        .append_event(&SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: event_id.clone(),
            producer_id: EventProducerId::from(producer),
            idempotency_key: IdempotencyKey::from(idempotency_key),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind(kind.to_owned()),
                kind_version: 1,
                correlation_id,
                causation_event_id,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
            },
        })
        .await?;
    Ok(event_id)
}

async fn complete_turn(
    store: &AgentSessionStore,
    session: &OpenSession,
    turn: &TurnStart,
    cause: EventId,
) -> Result<(), SampleEchoGateError> {
    append_event(
        store,
        &session.session_id,
        EventId::from(new_id("turn-completed")),
        "runtime-supervisor",
        new_id("turn-completed"),
        "turn/completed",
        CorrelationId::from(turn.operation.as_ref().to_owned()),
        Some(cause),
        json!({}),
    )
    .await?;
    Ok(())
}

async fn fail_turn(
    store: &AgentSessionStore,
    session: &OpenSession,
    turn: &TurnStart,
) -> Result<(), SampleEchoGateError> {
    append_event(
        store,
        &session.session_id,
        EventId::from(new_id("turn-failed")),
        "runtime-supervisor",
        new_id("turn-failed"),
        "turn/failed",
        CorrelationId::from(turn.operation.as_ref().to_owned()),
        Some(turn.turn_event_id.clone()),
        json!({"code": "PLUGIN_PANIC"}),
    )
    .await?;
    Ok(())
}

async fn run_broker_turn(
    store: Arc<AgentSessionStore>,
    registry: Arc<KernelRegistry>,
    compiled: CompiledSnapshot,
    session: &OpenSession,
) -> Result<(String, bool, usize), SampleEchoGateError> {
    let turn = begin_turn(&store, session, "Echo hello").await?;
    let (broker, transport) =
        recorded_broker(session.session_id.clone(), compiled.snapshot_ref().clone())?;
    let first = model_request(
        &session.session_id,
        compiled.snapshot_ref(),
        &turn.operation,
        &turn.user_event_id,
        vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text {
                text: "Echo hello".to_owned(),
            }],
            provider_round_id: None,
        }],
    );
    let first_events = broker
        .open_stream(first)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let call = first_events
        .iter()
        .find_map(|event| match &event.event {
            ChatModelEvent::ToolCallCompleted { call } => Some(call.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            SampleEchoGateError::Invariant("recorded Broker emitted no ToolCall".to_owned())
        })?;
    let execution =
        execute_tool(store.clone(), registry, compiled.clone(), session, &turn, call.clone())
            .await?;
    let ToolExecution::Succeeded {
        output,
        tool_result_event_id,
    } = execution
    else {
        return Err(SampleEchoGateError::Invariant(
            "clean sample.echo execution panicked".to_owned(),
        ));
    };
    let second = model_request(
        &session.session_id,
        compiled.snapshot_ref(),
        &turn.operation,
        &tool_result_event_id,
        vec![
            ChatMessage {
                role: ChatRole::Assistant,
                content: vec![ChatContentPart::ToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name,
                    arguments: call.arguments,
                    provider_metadata: None,
                }],
                provider_round_id: None,
            },
            ChatMessage {
                role: ChatRole::Tool,
                content: vec![ChatContentPart::ToolResult {
                    call_id: call.call_id,
                    output: vec![ChatToolResultPart::Text {
                        text: output.0.to_string(),
                    }],
                    is_error: false,
                }],
                provider_round_id: None,
            },
        ],
    );
    let second_events = broker
        .open_stream(second)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let text = second_events
        .iter()
        .filter_map(|event| match &event.event {
            ChatModelEvent::OutputTextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let message_correlation = CorrelationId::from(new_id("assistant-message"));
    let part_event = EventId::from(new_id("assistant-part"));
    append_event(
        &store,
        &session.session_id,
        part_event.clone(),
        "runtime-supervisor",
        new_id("assistant-part"),
        "message/content-part",
        message_correlation.clone(),
        Some(turn.turn_event_id.clone()),
        json!({"content": text}),
    )
    .await?;
    let completed_event = EventId::from(new_id("assistant-completed"));
    append_event(
        &store,
        &session.session_id,
        completed_event.clone(),
        "runtime-supervisor",
        new_id("assistant-completed"),
        "message/completed",
        message_correlation,
        Some(part_event),
        json!({
            "part_count": 1,
            "content_digest": digest_bytes(text.as_bytes())
        }),
    )
    .await?;
    complete_turn(&store, session, &turn, completed_event).await?;
    let cas_conflict = output
        .0
        .get("cas_conflict")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((text, cas_conflict, transport.calls()))
}

async fn run_direct_success_turn(
    store: Arc<AgentSessionStore>,
    registry: Arc<KernelRegistry>,
    compiled: CompiledSnapshot,
    session: &OpenSession,
    message: &str,
) -> Result<StrictJsonValue, SampleEchoGateError> {
    let turn = begin_turn(&store, session, message).await?;
    let call = ChatToolCall {
        call_id: ToolCallId(new_id("direct-call")),
        name: SAMPLE_CAPABILITY.to_owned(),
        arguments: StrictJsonValue(json!({"message": message})),
        provider_metadata: None,
    };
    match execute_tool(store.clone(), registry, compiled, session, &turn, call).await? {
        ToolExecution::Succeeded {
            output,
            tool_result_event_id,
        } => {
            complete_turn(&store, session, &turn, tool_result_event_id).await?;
            Ok(output)
        }
        ToolExecution::Panicked => Err(SampleEchoGateError::Invariant(
            "direct success turn panicked".to_owned(),
        )),
    }
}

async fn run_panic_turn(
    store: Arc<AgentSessionStore>,
    registry: Arc<KernelRegistry>,
    compiled: CompiledSnapshot,
    session: &OpenSession,
) -> Result<(), SampleEchoGateError> {
    let turn = begin_turn(&store, session, "panic").await?;
    let call = ChatToolCall {
        call_id: ToolCallId(new_id("panic-call")),
        name: SAMPLE_CAPABILITY.to_owned(),
        arguments: StrictJsonValue(json!({"message": "panic"})),
        provider_metadata: None,
    };
    match execute_tool(store.clone(), registry, compiled, session, &turn, call).await? {
        ToolExecution::Panicked => {
            fail_turn(&store, session, &turn).await?;
            Ok(())
        }
        ToolExecution::Succeeded { .. } => Err(SampleEchoGateError::Invariant(
            "panic fault unexpectedly succeeded".to_owned(),
        )),
    }
}

async fn dispose_and_delete(
    store: &AgentSessionStore,
    supervisor: &CodexRuntimeSupervisor,
    session: &OpenSession,
    expect_timeout: bool,
) -> Result<SampleEchoSessionReport, SampleEchoGateError> {
    let events = store
        .read_events(&session.session_id, None, 1000)
        .await?
        .events;
    let report = supervisor
        .dispose(RuntimeSessionDisposeParams {
            agent_session_id: session.session_id.clone(),
            runtime_binding_id: session.runtime_binding.runtime_binding_id.clone(),
            operation_id: OperationId::from(new_id("runtime-dispose")),
            reason: CanonicalErrorCode::from("SESSION_DELETE"),
        })
        .await?;
    if expect_timeout {
        invariant(
            report.rpc == DisposeRpcOutcome::TimedOut,
            "dispose timeout fault did not force cleanup",
        )?;
    } else {
        invariant(
            report.rpc == DisposeRpcOutcome::Acked,
            "normal Runtime dispose did not ACK",
        )?;
    }
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from(new_id("delete-session")),
        agent_session_id: session.session_id.clone(),
        owner_ref: session.binding_owner(),
        requested_at: now_ms(),
    };
    let deleted = store
        .delete_session(
            &command,
            now_ms().saturating_add(1),
        )
        .await?;
    invariant(
        store.inspect_tombstone(&session.session_id).await? == Some(deleted.tombstone),
        "D-024 tombstone could not be inspected",
    )?;
    supervisor
        .evict_disposed(&session.runtime_binding.runtime_binding_id)
        .await;
    Ok(SampleEchoSessionReport {
        agent_session_id: session.session_id.as_ref().to_owned(),
        revision: session.binding.preset_revision_ref.revision,
        persistent_session: true,
        effect_success_count: events
            .iter()
            .filter(|event| event.kind.0 == "effect/succeeded")
            .count() as u64,
        event_count_before_delete: events.len(),
        dispose_rpc: match report.rpc {
            DisposeRpcOutcome::Acked => "acked",
            DisposeRpcOutcome::TimedOut => "timed_out",
            DisposeRpcOutcome::Failed(_) => "failed",
        }
        .to_owned(),
        tombstone_committed: true,
    })
}

pub async fn run_sample_echo_gate(
    config: SampleEchoGateConfig,
) -> Result<SampleEchoGateReport, SampleEchoGateError> {
    invariant(
        config.working_root.is_absolute() && config.node_executable.is_absolute(),
        "sample.echo gate paths must be absolute",
    )?;
    let canonical_root = config.working_root.join("data");
    let bootstrap = FreshV4Coordinator::default()
        .bootstrap(&canonical_root, BUILD_IDENTITY, &[])
        .await?;
    let session_store = Arc::new(
        AgentSessionStore::connect_existing(
            bootstrap.canonical_root.join(FRESH_V4_DATABASE_FILE),
        )
        .await?,
    );
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let control = Arc::new(EchoControl::default());
    let registration = sample_registration("echo:", control.clone())?;
    let registry = Arc::new(KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        persistence.clone() as Arc<dyn PluginStatePersistence>,
    )?);
    let materialized = registry.replace_all(vec![registration.clone()])?;
    let package_materialized = materialized
        .packages
        .contains_key(&PackageId::from(SAMPLE_PACKAGE));
    let capability_materialized = materialized
        .capabilities
        .contains_key(&CapabilityId::from(SAMPLE_CAPABILITY));
    let skill_materialized = materialized.skills.contains_key(&SkillId::from(SAMPLE_SKILL));
    let mcp_materialized = materialized
        .mcp_for_capability(&CapabilityId::from(SAMPLE_CAPABILITY))
        .is_some();
    invariant(
        package_materialized
            && capability_materialized
            && skill_materialized
            && mcp_materialized,
        "Package/Capability/Skill/MCP materialization was incomplete",
    )?;
    let config_validated = materialized.plugins[&PluginMountId::from(SAMPLE_MOUNT)]
        .context
        .validated_config
        .value
        .0
        == json!({"prefix": "echo:"});

    let owner_id = UserId::from(Uuid::now_v7().to_string());
    let revisions = build_revisions(&materialized, &owner_id).await?;
    let owner_ref = owner(&owner_id);
    let clean_compiled = bind_canonical_snapshot(
        compile_kernel_snapshot(
            &materialized,
            revisions.clean_revision.clone(),
            owner_ref.clone(),
        )?,
        &revisions.clean_snapshot,
    )?;
    let dirty_compiled = bind_canonical_snapshot(
        compile_kernel_snapshot(
            &materialized,
            revisions.dirty_revision.clone(),
            owner_ref.clone(),
        )?,
        &revisions.dirty_snapshot,
    )?;

    let supervisor = Arc::new(CodexRuntimeSupervisor::new());
    let clean_session = open_persistent_session(
        session_store.clone(),
        supervisor.clone(),
        &materialized,
        &clean_compiled,
        &revisions.clean_revision,
        &owner_ref,
        &config.node_executable,
        &config.working_root.join("runtime-clean"),
        "ack",
    )
    .await?;
    let (first_echo, plugin_state_cas_conflict, broker_calls) = run_broker_turn(
        session_store.clone(),
        registry.clone(),
        clean_compiled,
        &clean_session,
    )
    .await?;
    let clean_report =
        dispose_and_delete(&session_store, &supervisor, &clean_session, false).await?;

    let generation_before_fault = registry.snapshot()?.generation;
    let mut invalid_registration = sample_registration("echo:", control.clone())?;
    invalid_registration.metadata.context.validated_config.value =
        StrictJsonValue(json!({"prefix": "echo:", "unknown": true}));
    let materialization_fault = registry.replace_all(vec![invalid_registration]).is_err();
    let materialization_failure_published_generation =
        registry.snapshot()?.generation != generation_before_fault;
    invariant(
        materialization_fault && !materialization_failure_published_generation,
        "materialization fault published a partial generation",
    )?;

    let dirty_session = open_persistent_session(
        session_store.clone(),
        supervisor.clone(),
        &materialized,
        &dirty_compiled,
        &revisions.dirty_revision,
        &owner_ref,
        &config.node_executable,
        &config.working_root.join("runtime-dirty"),
        "timeout",
    )
    .await?;
    let first_dirty = run_direct_success_turn(
        session_store.clone(),
        registry.clone(),
        dirty_compiled.clone(),
        &dirty_session,
        "before-restart",
    )
    .await?;
    let restarted_registry = Arc::new(KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        persistence.clone() as Arc<dyn PluginStatePersistence>,
    )?);
    let restarted_materialized =
        restarted_registry.replace_all(vec![sample_registration("echo:", control.clone())?])?;
    let restarted_compiled = bind_canonical_snapshot(
        compile_kernel_snapshot(
            &restarted_materialized,
            revisions.dirty_revision.clone(),
            owner_ref.clone(),
        )?,
        &revisions.dirty_snapshot,
    )?;
    invariant(
        restarted_compiled.snapshot_ref() == dirty_compiled.snapshot_ref(),
        "restart changed the frozen Snapshot",
    )?;
    let restart_output = run_direct_success_turn(
        session_store.clone(),
        restarted_registry.clone(),
        restarted_compiled.clone(),
        &dirty_session,
        "after-restart",
    )
    .await?;
    let plugin_state_survived_restart =
        first_dirty.0.get("count").and_then(Value::as_u64) == Some(1)
            && restart_output.0.get("count").and_then(Value::as_u64) == Some(2);
    control.panic_next.store(true, Ordering::Release);
    let effect_count_before_panic = control.successful_effects.load(Ordering::Acquire);
    run_panic_turn(
        session_store.clone(),
        restarted_registry,
        restarted_compiled,
        &dirty_session,
    )
    .await?;
    let effect_count_after_panic = control.successful_effects.load(Ordering::Acquire);
    let dirty_events = session_store
        .read_events(&dirty_session.session_id, None, 1000)
        .await?
        .events;
    let panic_effect_became_uncertain =
        dirty_events.iter().any(|event| event.kind.0 == "effect/uncertain");
    let panic_retried_effect = effect_count_after_panic != effect_count_before_panic;
    let dirty_report =
        dispose_and_delete(&session_store, &supervisor, &dirty_session, true).await?;
    let dispose_timeout_forced_tree_cleanup = dirty_report.dispose_rpc == "timed_out";
    let plugin_state_survived_session_delete = persistence
        .snapshot()?
        .entries()
        .any(|(identity, entry)| {
            identity.package_id.as_ref() == SAMPLE_PACKAGE
                && identity.state_key.as_ref() == "invoke-count"
                && entry.value.0.as_u64() == Some(2)
        });

    invariant(
        revisions
            .store
            .get_revision_number(
                &revisions.dirty_revision.reference.preset_id,
                revisions.dirty_revision.reference.revision,
            )
            .await?
            .is_some(),
        "ordinary dirty Revision disappeared",
    )?;
    let _ = revisions.control_plane.catalog()?;
    Ok(SampleEchoGateReport {
        package_materialized,
        capability_materialized,
        skill_materialized,
        mcp_materialized,
        config_validated,
        clean_revision_action: revision_action_name(revisions.clean_action),
        dirty_revision_action: revision_action_name(revisions.dirty_action),
        clean_revision: revisions.clean_revision.reference.revision,
        dirty_revision: revisions.dirty_revision.reference.revision,
        clean_session: clean_report,
        dirty_session: dirty_report,
        broker_recorded_transport_calls: broker_calls,
        first_echo,
        restart_echo: restart_output
            .0
            .get("echo")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        plugin_state_cas_conflict,
        plugin_state_survived_restart,
        plugin_state_survived_session_delete,
        faults: SampleEchoFaultReport {
            save_failure_created_revision: !revisions.save_fault_rejected,
            save_failure_created_session: false,
            materialization_failure_published_generation,
            panic_effect_became_uncertain,
            panic_retried_effect,
            dispose_timeout_forced_tree_cleanup,
        },
    })
}

fn revision_action_name(action: EditorRevisionActionDto) -> String {
    match action {
        EditorRevisionActionDto::ReuseCurrentRevision => "reuse_current_revision",
        EditorRevisionActionDto::SaveOrdinaryVisibleRevision => {
            "save_ordinary_visible_revision"
        }
    }
    .to_owned()
}

fn invariant(condition: bool, message: &str) -> Result<(), SampleEchoGateError> {
    if condition {
        Ok(())
    } else {
        Err(SampleEchoGateError::Invariant(message.to_owned()))
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
