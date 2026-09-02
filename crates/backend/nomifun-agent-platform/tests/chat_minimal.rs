use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, ArtifactId, CanonicalErrorCode, CompactionCompletedPayload,
    ChatRouteIdentity, ConnectionConfigRef, CorrelationId, DeleteAgentSessionCommand, DigestHex,
    EventId,
    EventProducerId, FullAutoExecutionWire, IdempotencyKey, LogicalArtifactRef, ModelRouteId,
    NativeActionStart, NativeActionStartAck, OperationId, PackageId, PrincipalRef,
    RuntimeBindingContract,
    RuntimeBindingId, RuntimeCancelParams, RuntimeCommand, RuntimeCommandContext,
    RuntimeCreateParams, RuntimeEventAck, RuntimeEventEnvelope, RuntimeHelloPayload,
    RuntimeReleaseTargetPayload, RuntimeResumeParams, RuntimeSessionDisposeParams,
    RuntimeStartTurnParams, SemanticSessionEventDraft, SessionEventAck, SessionEventAppend,
    SessionEventKind, SessionEventPayloadRef, SessionPayloadBody, SessionPayloadRecord,
    ChatRouteLookupKey, StrictJsonValue, UserId, VersionString, canonical_json_bytes, digest_bytes,
    digest_payload, official_preset_seed_manifest_payload, AGENT_CORE_PACKAGE_ID,
};
use nomifun_agent_control_plane::{
    CompilerReleaseInputs, ControlPlaneStore,
};
use nomifun_agent_kernel::{
    CompiledSnapshot, CompilerEnvironment, MaterializationPolicy, SessionCapabilityState,
};
use nomifun_agent_platform::{
    load_exact_chat_route_record, AgentPlatform, AgentPlatformConfig, ChatMinimalContract,
    ChatMinimalHiddenInitialization,
};
use nomifun_agent_session::{
    CreateSessionRequest, RuntimeAppendContext, SessionStoreError, ZeroOutstandingProof,
};
use nomifun_api_types::{
    CreateAgentPresetFromTemplateRequest, ResolveSavedRevisionPreviewRequest,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatCausality, ChatCausalityGate,
    ChatContentPart, ChatMessage, ChatModelBroker, ChatModelError, ChatModelErrorCode,
    ChatModelEvent, ChatModelInput, ChatModelRequest, ChatModality, ChatProtocol,
    ChatProtocolAdapter, ChatResponseFormat, ChatRetryDirective, ChatRole, ChatRouteResolver,
    ChatRouteSelection, ChatToolChoice, CredentialLease, CredentialTarget,
    GeminiAdapter, OpenAiChatAdapter, OpenAiResponsesAdapter, PromptCachePolicy,
    ProviderCredentialRef, ProviderCredentialStore, ProviderIdRef, ProviderTransport,
    ProviderWireFrame, ProviderWireRequest, ProviderWireStream, ResolvedChatRoute,
    ResolvedChatRouteSet, VertexAdapter, protocol_features,
};
use nomifun_codex_runtime::{
    ClientLimits, CodexRuntimeSupervisor, DisposeRpcOutcome, InheritedHandleCredential,
    RuntimeError, RuntimeHelloExpectation, RuntimeIngressPort, RuntimeLaunchRequest,
    RuntimeProcessConfig, RuntimeReleaseDescriptor,
};
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FreshV4Coordinator, canonical_schema_manifest_digest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

const VERSION: &str = "1.0.0";
const BUILD_IDENTITY: &str = "c6-chat-minimal-2026-08-29";
const MODEL_ROUTE: &str = "chat-minimal-recorded";
const MODEL_ROUTE_REVISION: u64 = 1;
const RESPONSE_TEXT: &str = "Hello from chat.minimal.";
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
    case "create":
    case "resume": {
      const serverId = "runtime-bound:" + message.id;
      pending.set(serverId, { client_id: message.id, result: config.binding });
      send({ id: serverId, method: "runtime/event", params: config.runtime_bound_event });
      break;
    }
    case "start_turn":
      send({ id: message.id, result: { accepted: true } });
      break;
    case "cancel":
      send({ id: message.id, result: {} });
      break;
    case "session_dispose":
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

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
struct ExpectedCausality {
    agent_session_id: AgentSessionId,
    snapshot_digest: DigestHex,
}

#[derive(Default)]
struct ExactCausalityGate {
    expected: Mutex<Option<ExpectedCausality>>,
    calls: AtomicUsize,
}

impl ExactCausalityGate {
    fn bind(&self, agent_session_id: AgentSessionId, snapshot_digest: DigestHex) {
        *self.expected.lock().expect("causality lock") = Some(ExpectedCausality {
            agent_session_id,
            snapshot_digest,
        });
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ChatCausalityGate for ExactCausalityGate {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let expected = self
            .expected
            .lock()
            .expect("causality lock")
            .clone()
            .ok_or_else(|| {
                ChatModelError::new(
                    ChatModelErrorCode::CausalityRejected,
                    "chat.minimal causality was not bound",
                    ChatRetryDirective::Never,
                )
            })?;
        if causality.agent_session_id != expected.agent_session_id
            || causality.resolved_snapshot_ref.snapshot_digest != expected.snapshot_digest
        {
            return Err(ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                "chat.minimal request escaped its Session or frozen Snapshot",
                ChatRetryDirective::Never,
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
            return Err(ChatModelError::new(
                ChatModelErrorCode::RouteRevisionMismatch,
                "chat.minimal route selection changed",
                ChatRetryDirective::Never,
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
            "chat-minimal-recorded-handle",
        ))
    }
}

struct RecordedProviderTransport {
    scripts: Mutex<VecDeque<Vec<ProviderWireFrame>>>,
    requests: Mutex<Vec<ProviderWireRequest>>,
    calls: AtomicUsize,
}

impl RecordedProviderTransport {
    fn new(scripts: impl IntoIterator<Item = Vec<ProviderWireFrame>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }

    fn requests(&self) -> Vec<ProviderWireRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait]
impl ProviderTransport for RecordedProviderTransport {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        if !credential.validates_route(&recorded_route())
            || credential.credential_ref() != &request.credential_ref
        {
            return Err(ChatModelError::protocol_violation(
                "recorded credential target mismatch",
            ));
        }
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.requests
            .lock()
            .expect("request lock")
            .push(request);
        let frames = self
            .scripts
            .lock()
            .expect("script lock")
            .pop_front()
            .ok_or_else(|| ChatModelError::provider_unavailable("recorded stream exhausted"))?;
        Ok(Box::pin(stream::iter(frames.into_iter().map(Ok))))
    }
}

struct SessionRuntimeIngress {
    platform: Arc<AgentPlatform>,
    agent_session_id: AgentSessionId,
}

#[async_trait]
impl RuntimeIngressPort for SessionRuntimeIngress {
    async fn append_runtime_event(
        &self,
        event: RuntimeEventEnvelope,
    ) -> Result<RuntimeEventAck, RuntimeError> {
        self.platform
            .session_store()
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id: self.agent_session_id.clone(),
                envelope: event,
            })
            .await
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
            .ack
            .ok_or_else(|| RuntimeError::Protocol("runtime event was not persisted".to_owned()))
    }

    async fn commit_native_action_start(
        &self,
        _start: NativeActionStart,
    ) -> Result<NativeActionStartAck, RuntimeError> {
        Err(RuntimeError::Protocol(
            "chat.minimal cannot dispatch native actions".to_owned(),
        ))
    }
}

struct RuntimeFixture {
    release: RuntimeReleaseDescriptor,
    hello: RuntimeHelloPayload,
    target_id: String,
}

struct OpenRuntime {
    binding: RuntimeBindingContract,
}

#[tokio::test]
async fn chat_minimal_runs_the_formal_final_stack() -> TestResult<()> {
    let directory = tempfile::tempdir()?;
    let canonical_root = directory.path().join("data");
    let bootstrap = FreshV4Coordinator::default()
        .bootstrap(&canonical_root, BUILD_IDENTITY, &[])
        .await?;
    let pool = open_pool(
        &bootstrap.canonical_root.join(FRESH_V4_DATABASE_FILE),
    )
    .await?;

    let contract = ChatMinimalContract::frozen()?;
    let seed_manifest = official_preset_seed_manifest_payload();
    let release_inputs = CompilerReleaseInputs {
        resolver_version: VersionString::from(VERSION),
        runtime_protocol_version: VersionString::from(VERSION),
        runtime_feature_inventory_digest: seed_manifest
            .target_runtime_feature_inventory_digest
            .clone(),
        canonical_schema_manifest_digest: canonical_schema_manifest_digest()?,
        target_contribution_manifest_digest: seed_manifest
            .target_first_party_contribution_digest
            .clone(),
        availability_evidence_revision: BUILD_IDENTITY.to_owned(),
    };
    let kernel_environment = CompilerEnvironment {
        resolver_version: VersionString::from(VERSION),
        required_runtime_protocol_version: VersionString::from(VERSION),
        required_runtime_profile: nomifun_agent_contracts::RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: release_inputs
            .runtime_feature_inventory_digest
            .clone(),
        available_runtime_features: BTreeSet::new(),
        canonical_schema_manifest_digest: release_inputs
            .canonical_schema_manifest_digest
            .clone(),
        target_contribution_manifest_digest: release_inputs
            .target_contribution_manifest_digest
            .clone(),
        host_target: nomifun_agent_contracts::RuntimeTarget::from(native_target_id()),
        host_surface: "desktop".to_owned(),
        availability_evidence_revision: BUILD_IDENTITY.to_owned(),
    };

    let route = recorded_route();
    let causality_gate = Arc::new(ExactCausalityGate::default());
    let transport = RecordedProviderTransport::new([recorded_frames()]);
    let broker = recorded_broker(
        Arc::clone(&causality_gate),
        route.clone(),
        Arc::clone(&transport),
    )?;
    let supervisor = Arc::new(CodexRuntimeSupervisor::new());
    let platform = AgentPlatform::from_pool(AgentPlatformConfig::with_supervisor(
        pool.clone(),
        MaterializationPolicy::stable(VERSION),
        release_inputs,
        kernel_environment,
        Arc::clone(&supervisor),
        broker,
    ))
    .await?;

    let owner = UserId::from(Uuid::now_v7().to_string());
    let owner_ref = user_principal(&owner);
    let library = platform.control_plane().library(&owner).await?;
    let template = library
        .official_templates
        .iter()
        .find(|template| {
            template.template_key == nomifun_api_types::OfficialPresetKeyDto::ChatMinimal
        })
        .expect("official chat.minimal template");
    contract.validate_template(template)?;

    let editor = platform
        .control_plane()
        .create_from_template(
            &owner,
            "chat.minimal",
            CreateAgentPresetFromTemplateRequest {
                display_name: "Minimal Chat".to_owned(),
                description: None,
                resource_bindings: Vec::new(),
                model_route_refs: BTreeMap::from([(
                    "agent_chat".to_owned(),
                    MODEL_ROUTE.to_owned(),
                )]),
                chat_route_records: BTreeMap::from([(
                    "agent_chat".to_owned(),
                    json!({
                        "schema": "nomifun.chat-route-record.v1",
                        "task": "agent_chat",
                        "primary": {
                            "model_route_id": MODEL_ROUTE,
                            "model_route_revision": MODEL_ROUTE_REVISION,
                            "provider_id": "provider-chat-minimal",
                            "model": "chat-minimal-recorded-model",
                            "protocol": "openai_chat",
                            "connection_config_ref": "connection-chat-minimal",
                            "config_revision_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "credential_ref": "credential-chat-minimal",
                            "features": [
                                "text_input",
                                "text_output",
                                "tool_calls",
                                "reasoning",
                                "image_input",
                                "audio_input",
                                "structured_output"
                            ]
                        },
                        "failovers": []
                    }),
                )]),
            },
        )
        .await?;
    contract.validate_ordinary_revision(&editor)?;
    let revision_dto = editor.revision.as_ref().expect("ordinary Revision");
    let preview = platform
        .control_plane()
        .preview_saved_revision(
            &owner,
            &editor.preset.preset_id,
            revision_dto.reference.revision,
            ResolveSavedRevisionPreviewRequest {
                scene: "chat".to_owned(),
                surface: "desktop".to_owned(),
                audience: "owner".to_owned(),
            },
        )
        .await?;
    contract.validate_preview(&preview)?;

    let preset_id = AgentPresetId::from(editor.preset.preset_id.clone());
    let revision = platform
        .control_store()
        .get_revision_number(&preset_id, revision_dto.reference.revision)
        .await?
        .expect("persisted Revision");
    let snapshot = platform
        .control_store()
        .get_snapshot(&revision.reference)
        .await?
        .expect("persisted ResolvedSnapshot");
    let route_json: String = sqlx::query_scalar(
        "SELECT route_json FROM agent_preset_model_routes \
         WHERE revision_id = ? AND model_task = ?",
    )
    .bind(format!(
        "{}@{}",
        revision.reference.preset_id.as_ref(),
        revision.reference.revision
    ))
    .bind("agent_chat")
    .fetch_one(&pool)
    .await?;
    let route_value: Value = serde_json::from_str(&route_json)?;
    assert!(route_value.is_object());
    assert_eq!(
        route_value["primary"]["model_route_id"],
        json!(MODEL_ROUTE)
    );
    let route_lookup = ChatRouteLookupKey {
        preset_revision_id: format!(
            "{}@{}",
            revision.reference.preset_id.as_ref(),
            revision.reference.revision
        ),
        model_task: "agent_chat".to_owned(),
        route_id: ModelRouteId::from(MODEL_ROUTE),
        route_revision: MODEL_ROUTE_REVISION,
    };
    let persisted_route = load_exact_chat_route_record(&pool, &route_lookup)
        .await?
        .expect("exact route record");
    assert_eq!(
        persisted_route.primary.model_route_id.as_ref(),
        MODEL_ROUTE
    );
    let mut wrong_route_revision = route_lookup.clone();
    wrong_route_revision.route_revision += 1;
    assert!(
        load_exact_chat_route_record(&pool, &wrong_route_revision)
            .await
            .is_err()
    );
    let mut wrong_revision_id = route_lookup.clone();
    wrong_revision_id.preset_revision_id.push_str("-other");
    assert!(
        load_exact_chat_route_record(&pool, &wrong_revision_id)
            .await?
            .is_none()
    );
    contract.validate_snapshot(&snapshot.content)?;
    let registry = platform.materialized_registry()?;
    assert_eq!(
        registry.packages.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([PackageId::from(AGENT_CORE_PACKAGE_ID)])
    );
    assert!(registry.capabilities.is_empty());
    assert!(registry.skills.is_empty());
    assert!(registry.mcp_tools.is_empty());
    let core_services = registry
        .service_dag
        .nodes
        .iter()
        .find(|node| node.package.id.as_ref() == AGENT_CORE_PACKAGE_ID)
        .expect("chat.minimal retains the canonical Session service provider");
    assert_eq!(core_services.provides.len(), 2);
    assert!(core_services.requires.is_empty());
    let compiled = CompiledSnapshot {
        envelope: snapshot.clone(),
        authority_policies: BTreeMap::new(),
        registry_generation: registry.generation,
        registry_digest: registry.registry_digest.clone(),
    };
    let capability_state = SessionCapabilityState::new(&compiled);
    let active = capability_state.snapshot()?;
    assert!(active.active.is_empty());
    assert!(capability_state.search("", 32)?.is_empty());
    let profile = platform.pinned_runtime_profile(&compiled);
    contract.validate_runtime_profile(&profile)?;
    contract.validate_hidden_initialization(&hidden_initialization(
        &registry,
        &profile,
        directory.path(),
    ))?;

    let session_id = AgentSessionId::from(Uuid::now_v7().to_string());
    let binding = AgentBindingValue {
        preset_revision_ref: revision.reference.clone(),
        resolved_snapshot_ref: snapshot.snapshot_ref.clone(),
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let created = platform
        .session_store()
        .create_session(CreateSessionRequest {
            session: AgentSessionLiveRecord {
                agent_session_id: session_id.clone(),
                owner_ref: owner_ref.clone(),
                metadata: AgentSessionMetadata {
                    title: Some("Minimal Chat".to_owned()),
                    archived: false,
                    pinned: false,
                },
                agent_binding: binding,
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            created_at: now_ms(),
            operation_id: OperationId::from(new_id("session-open")),
            producer_id: EventProducerId::from("session-api"),
            idempotency_key: IdempotencyKey::from(new_id("session-open")),
            correlation_id: CorrelationId::from(new_id("session")),
            initial_input: None,
            opening_event_id: Some(EventId::from(new_id("session-opening"))),
            activation_event_id: Some(EventId::from(new_id("active-set"))),
            initial_active_capability_ids: Vec::new(),
        })
        .await?;
    assert_eq!(created.opening_ack.seq, 1);
    assert_eq!(created.activation_ack.seq, 2);

    let node = find_node().ok_or("Node.js is required for the managed stdio runtime fixture")?;
    let first_runtime = launch_runtime(
        Arc::clone(&platform),
        &contract,
        &compiled,
        &node,
        &directory.path().join("runtime-create"),
        &session_id,
        &created.opening_ack.event_id,
        created.activation_ack.seq,
        false,
        None,
    )
    .await?;
    let ready = append_event(
        &platform,
        &session_id,
        "runtime-supervisor",
        "session/ready",
        CorrelationId::from(session_id.as_ref().to_owned()),
        Some(first_runtime.binding.runtime_bound_event_id.clone()),
        json!({
            "runtime_binding_id": first_runtime.binding.runtime_binding_id
        }),
    )
    .await?;
    causality_gate.bind(
        session_id.clone(),
        snapshot.snapshot_ref.snapshot_digest.clone(),
    );

    let turn_operation = OperationId::from(new_id("turn"));
    let turn_started = append_event(
        &platform,
        &session_id,
        "session-api",
        "turn/started",
        CorrelationId::from(turn_operation.as_ref().to_owned()),
        Some(ready.event_id.clone()),
        json!({"operation_id": turn_operation}),
    )
    .await?;
    let user_message = append_event(
        &platform,
        &session_id,
        "session-api",
        "message/user-accepted",
        CorrelationId::from(new_id("user-message")),
        Some(turn_started.event_id.clone()),
        json!({"content": "Say hello."}),
    )
    .await?;
    platform
        .runtime_port()
        .command(
            &first_runtime.binding.runtime_binding_id,
            &RuntimeCommand::StartTurn(RuntimeStartTurnParams {
                context: runtime_context(
                    &session_id,
                    &first_runtime.binding.runtime_binding_id,
                    turn_operation.clone(),
                    &compiled,
                ),
                idempotency_key: IdempotencyKey::from(new_id("turn")),
                input_event_id: user_message.event_id.clone(),
            }),
        )
        .await?;

    let model_request = chat_request(
        &session_id,
        &compiled,
        &turn_operation,
        &user_message.event_id,
        &route,
    );
    contract.validate_model_request(
        &model_request,
        &session_id,
        &snapshot.snapshot_ref.snapshot_digest,
    )?;
    let mut stream = platform.open_model_stream(model_request).await?;
    let mut model_events = Vec::new();
    while let Some(event) = stream.next().await {
        model_events.push(event.map_err(model_error)?.event);
    }
    let streamed_text = contract.validate_model_stream(&model_events)?;
    assert_eq!(streamed_text, RESPONSE_TEXT);

    let message_correlation = CorrelationId::from(new_id("assistant-message"));
    let mut last_part = turn_started.event_id.clone();
    let mut part_count = 0_u64;
    for event in &model_events {
        if let ChatModelEvent::OutputTextDelta { text } = event {
            let ack = append_event(
                &platform,
                &session_id,
                "runtime-supervisor",
                "message/content-part",
                message_correlation.clone(),
                Some(last_part.clone()),
                json!({"content": text}),
            )
            .await?;
            last_part = ack.event_id;
            part_count += 1;
        }
    }
    let message_completed = append_event(
        &platform,
        &session_id,
        "runtime-supervisor",
        "message/completed",
        message_correlation,
        Some(last_part),
        json!({
            "content_digest": digest_bytes(streamed_text.as_bytes()),
            "part_count": part_count
        }),
    )
    .await?;
    let turn_completed = append_event(
        &platform,
        &session_id,
        "runtime-supervisor",
        "turn/completed",
        CorrelationId::from(turn_operation.as_ref().to_owned()),
        Some(turn_started.event_id),
        json!({"message_event_id": message_completed.event_id}),
    )
    .await?;
    let successful = platform
        .session_store()
        .observe(&session_id, None, 500)
        .await?;
    contract.validate_success_observation(&successful, RESPONSE_TEXT)?;
    assert_eq!(causality_gate.calls(), 1);
    assert_eq!(transport.calls(), 1);
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .body
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );

    let cancelled_operation = OperationId::from(new_id("cancelled-turn"));
    let cancelled_started = append_event(
        &platform,
        &session_id,
        "session-api",
        "turn/started",
        CorrelationId::from(cancelled_operation.as_ref().to_owned()),
        Some(turn_completed.event_id.clone()),
        json!({"operation_id": cancelled_operation}),
    )
    .await?;
    let cancelled_input = append_event(
        &platform,
        &session_id,
        "session-api",
        "message/user-accepted",
        CorrelationId::from(new_id("cancelled-input")),
        Some(cancelled_started.event_id.clone()),
        json!({"content": "Cancel this turn."}),
    )
    .await?;
    let cancelled_context = runtime_context(
        &session_id,
        &first_runtime.binding.runtime_binding_id,
        cancelled_operation.clone(),
        &compiled,
    );
    platform
        .runtime_port()
        .command(
            &first_runtime.binding.runtime_binding_id,
            &RuntimeCommand::StartTurn(RuntimeStartTurnParams {
                context: cancelled_context.clone(),
                idempotency_key: IdempotencyKey::from(new_id("cancelled-turn")),
                input_event_id: cancelled_input.event_id,
            }),
        )
        .await?;
    platform
        .runtime_port()
        .command(
            &first_runtime.binding.runtime_binding_id,
            &RuntimeCommand::Cancel(RuntimeCancelParams {
                context: cancelled_context,
                target_operation_id: cancelled_operation.clone(),
            }),
        )
        .await?;
    append_event(
        &platform,
        &session_id,
        "session-api",
        "turn/cancelled",
        CorrelationId::from(cancelled_operation.as_ref().to_owned()),
        Some(cancelled_started.event_id),
        json!({"operation_id": cancelled_operation}),
    )
    .await?;
    let cancelled = platform
        .session_store()
        .observe(&session_id, None, 500)
        .await?;
    contract.validate_cancel_observation(&cancelled, cancelled_operation.as_ref())?;
    assert_eq!(transport.calls(), 1, "cancelled turn reached the Broker");

    let compaction_value = json!({
        "summary": "User asked for a greeting and received the minimal response."
    });
    let compaction_bytes = canonical_json_bytes(&compaction_value)?;
    let compaction_payload = SessionPayloadRecord {
        payload_id: ArtifactId::from(new_id("compaction-payload")),
        agent_session_id: session_id.clone(),
        media_type: "application/json".to_owned(),
        byte_len: compaction_bytes.len() as u64,
        digest: digest_bytes(&compaction_bytes),
        body: SessionPayloadBody::Json(StrictJsonValue(compaction_value)),
    };
    let compaction = CompactionCompletedPayload {
        agent_session_id: session_id.clone(),
        through_seq: turn_completed.seq,
        context_payload_id: compaction_payload.payload_id.clone(),
        context_digest: compaction_payload.digest.clone(),
    };
    let compaction_event = SessionEventAppend {
        agent_session_id: session_id.clone(),
        event_id: EventId::from(new_id("compaction-completed")),
        producer_id: EventProducerId::from("compaction-coordinator"),
        idempotency_key: IdempotencyKey::from(new_id("compaction-completed")),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("compaction/completed".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId::from(new_id("compaction")),
            causation_event_id: Some(turn_completed.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                serde_json::to_value(&compaction)?,
            )),
        },
    };
    platform
        .session_store()
        .append_event_with_payload(&compaction_event, Some(&compaction_payload))
        .await?;
    let rehydration = platform
        .session_store()
        .rehydration_input(&session_id)
        .await?;
    contract.validate_rehydration(
        &rehydration,
        &session_id,
        &snapshot.snapshot_ref.snapshot_digest,
    )?;

    dispose_runtime(
        &platform,
        &first_runtime.binding,
        "RUNTIME_REBIND",
    )
    .await?;
    let resumed_runtime = launch_runtime(
        Arc::clone(&platform),
        &contract,
        &compiled,
        &node,
        &directory.path().join("runtime-resume"),
        &session_id,
        &created.opening_ack.event_id,
        rehydration.through_cursor.seq,
        true,
        Some(digest_payload(&rehydration)?),
    )
    .await?;
    assert_ne!(
        first_runtime.binding.runtime_binding_id,
        resumed_runtime.binding.runtime_binding_id
    );

    let before_rebuild = platform
        .session_store()
        .observe(&session_id, None, 500)
        .await?;
    platform
        .session_store()
        .rebuild_projections(&session_id)
        .await?;
    let after_rebuild = platform
        .session_store()
        .observe(&session_id, None, 500)
        .await?;
    contract.validate_rebuild(&before_rebuild, &after_rebuild)?;

    dispose_runtime(
        &platform,
        &resumed_runtime.binding,
        "SESSION_DELETE",
    )
    .await?;
    assert_eq!(supervisor.session_count().await, 0);
    let delete_command = DeleteAgentSessionCommand {
        operation_id: OperationId::from(new_id("delete-session")),
        agent_session_id: session_id.clone(),
        owner_ref: owner_ref.clone(),
        requested_at: now_ms(),
    };
    let deleted = platform
        .session_store()
        .delete_session(
            &delete_command,
            &ZeroOutstandingProof::verified(),
            delete_command.requested_at.saturating_add(1),
        )
        .await?;
    contract.validate_delete(&deleted, &session_id, &owner_ref)?;
    assert_eq!(
        platform
            .session_store()
            .inspect_tombstone(&session_id)
            .await?,
        Some(deleted.tombstone)
    );

    let observe_error = platform
        .session_store()
        .observe(&session_id, None, 10)
        .await
        .expect_err("deleted Session cannot be observed");
    contract.validate_deleted_error(observe_error.code())?;
    let resume_error = platform
        .session_store()
        .rehydration_input(&session_id)
        .await
        .expect_err("deleted Session cannot resume");
    contract.validate_deleted_error(resume_error.code())?;
    let rebuild_error = platform
        .session_store()
        .rebuild_projections(&session_id)
        .await
        .expect_err("deleted Session cannot rebuild projections");
    contract.validate_deleted_error(rebuild_error.code())?;
    let late_error = platform
        .session_store()
        .append_event(&SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(new_id("late-event")),
            producer_id: EventProducerId::from("session-api"),
            idempotency_key: IdempotencyKey::from(new_id("late-event")),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/started".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(new_id("late-turn")),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
            },
        })
        .await
        .expect_err("late event cannot reopen a deleted Session");
    contract.validate_deleted_error(late_error.code())?;
    assert!(matches!(late_error, SessionStoreError::Deleted(_)));
    Ok(())
}

#[test]
fn chat_minimal_has_no_legacy_or_mock_runtime_shortcut() {
    let source = include_str!("../src/chat.rs");
    for forbidden in [
        "Conversation",
        "Factory",
        "GatewayDeps",
        "AppServices",
        "MockRuntime",
        "mock_runtime",
        "DraftSnapshot",
        "ephemeral_session",
        "hidden_revision",
        "AwaitingApproval",
        "Confirmation",
    ] {
        assert!(
            !source.contains(forbidden),
            "chat.minimal reintroduced forbidden surface {forbidden}"
        );
    }
}

fn recorded_route() -> ResolvedChatRoute {
    ResolvedChatRoute {
        model_route_id: ModelRouteId::from(MODEL_ROUTE),
        model_route_revision: MODEL_ROUTE_REVISION,
        provider_id: ProviderIdRef::from("provider-chat-minimal"),
        model: "chat-minimal-recorded-model".to_owned(),
        protocol: ChatProtocol::OpenaiChat,
        connection_config_ref: ConnectionConfigRef::from("connection-chat-minimal"),
        config_revision_digest: DigestHex::from("b".repeat(64)),
        credential_ref: ProviderCredentialRef::from("credential-chat-minimal"),
        features: protocol_features(ChatProtocol::OpenaiChat),
    }
}

fn recorded_frames() -> Vec<ProviderWireFrame> {
    vec![
        ProviderWireFrame {
            event: "response.start".to_owned(),
            data: json!({"id": "chat-minimal-response"}),
        },
        ProviderWireFrame {
            event: "text.delta".to_owned(),
            data: json!({"text": "Hello from "}),
        },
        ProviderWireFrame {
            event: "text.delta".to_owned(),
            data: json!({"text": "chat.minimal."}),
        },
        ProviderWireFrame {
            event: "usage".to_owned(),
            data: json!({"input_tokens": 4, "output_tokens": 4}),
        },
        ProviderWireFrame {
            event: "done".to_owned(),
            data: json!({"finish_reason": "stop"}),
        },
    ]
}

fn recorded_broker(
    causality_gate: Arc<ExactCausalityGate>,
    route: ResolvedChatRoute,
    transport: Arc<RecordedProviderTransport>,
) -> TestResult<Arc<ChatModelBroker>> {
    let provider_transport: Arc<dyn ProviderTransport> = transport;
    let adapters: Vec<Arc<dyn ChatProtocolAdapter>> = vec![
        Arc::new(AnthropicAdapter::new(Arc::clone(&provider_transport))),
        Arc::new(OpenAiChatAdapter::new(Arc::clone(&provider_transport))),
        Arc::new(OpenAiResponsesAdapter::new(Arc::clone(
            &provider_transport,
        ))),
        Arc::new(GeminiAdapter::new(Arc::clone(&provider_transport))),
        Arc::new(BedrockAdapter::new(Arc::clone(&provider_transport))),
        Arc::new(VertexAdapter::new(provider_transport)),
    ];
    Ok(Arc::new(
        ChatModelBroker::new(
            causality_gate,
            Arc::new(ExactRouteResolver { route }),
            Arc::new(ExactCredentialStore),
            adapters,
            BrokerRetryPolicy::default(),
        )
        .map_err(model_error)?,
    ))
}

fn chat_request(
    session_id: &AgentSessionId,
    compiled: &CompiledSnapshot,
    turn_operation_id: &OperationId,
    causation_event_id: &EventId,
    route: &ResolvedChatRoute,
) -> ChatModelRequest {
    let route_identity = ChatRouteIdentity::new(
        compiled.content().preset_revision_ref.revision_id(),
        nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
        route.model_route_id.clone(),
        route.model_route_revision,
    );
    ChatModelRequest {
        contract_version: VersionString::from("chat-model-v1"),
        causality: ChatCausality {
            agent_session_id: session_id.clone(),
            turn_operation_id: turn_operation_id.clone(),
            causation_event_id: causation_event_id.clone(),
            resolved_snapshot_ref: compiled.snapshot_ref().clone(),
            route_identity: route_identity.clone(),
            operation_id: OperationId::from(new_id("model")),
        },
        route: route_identity,
        input: ChatModelInput {
            instructions: vec!["Answer directly and concisely.".to_owned()],
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: vec![ChatContentPart::Text {
                    text: "Say hello.".to_owned(),
                }],
                provider_round_id: None,
            }],
            tools: Vec::new(),
            tool_choice: ChatToolChoice::None,
            max_output_tokens: Some(128),
            reasoning: None,
            prompt_cache: PromptCachePolicy::Disabled,
            response_format: ChatResponseFormat::Text,
            requested_output_modalities: BTreeSet::from([ChatModality::Text]),
            provider_round_parent: None,
            preserve_native_responses_items: false,
            metadata: BTreeMap::new(),
        },
    }
}

async fn open_pool(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
}

async fn append_event(
    platform: &AgentPlatform,
    session_id: &AgentSessionId,
    producer: &str,
    kind: &str,
    correlation_id: CorrelationId,
    causation_event_id: Option<EventId>,
    payload: Value,
) -> Result<SessionEventAck, SessionStoreError> {
    platform
        .session_store()
        .append_event(&SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(new_id(kind)),
            producer_id: EventProducerId::from(producer),
            idempotency_key: IdempotencyKey::from(new_id(kind)),
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
        .await?
        .ack
        .ok_or_else(|| SessionStoreError::InvalidEvent(format!("{kind} did not persist")))
}

fn runtime_context(
    session_id: &AgentSessionId,
    runtime_binding_id: &RuntimeBindingId,
    operation_id: OperationId,
    compiled: &CompiledSnapshot,
) -> RuntimeCommandContext {
    RuntimeCommandContext {
        agent_session_id: session_id.clone(),
        runtime_binding_id: runtime_binding_id.clone(),
        operation_id,
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        runtime_profile_digest: compiled
            .content()
            .compiled_runtime_profile_digest
            .clone(),
        active_set_generation: 0,
    }
}

#[allow(clippy::too_many_arguments)]
async fn launch_runtime(
    platform: Arc<AgentPlatform>,
    contract: &ChatMinimalContract,
    compiled: &CompiledSnapshot,
    node_executable: &Path,
    runtime_directory: &Path,
    session_id: &AgentSessionId,
    opening_event_id: &EventId,
    through_seq: u64,
    resume: bool,
    compatibility_digest: Option<DigestHex>,
) -> TestResult<OpenRuntime> {
    let runtime_binding_id = RuntimeBindingId::from(new_id("runtime-binding"));
    let runtime_bound_event_id = EventId::from(new_id("runtime-bound"));
    let placeholder_release = RuntimeReleaseDescriptor::frozen_from_fixture()?;
    let mut binding = RuntimeBindingContract {
        runtime_binding_id: runtime_binding_id.clone(),
        agent_session_id: session_id.clone(),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        runtime_release_digest: placeholder_release.payload_digest,
        runtime_build_digest: DigestHex::from("0".repeat(64)),
        protocol_version: VersionString::from(VERSION),
        profile_kind: nomifun_agent_contracts::RuntimeProfileKind::ManagedMinimal,
        runtime_profile_digest: compiled
            .content()
            .compiled_runtime_profile_digest
            .clone(),
        active_set_generation: 0,
        runtime_bound_event_id,
        through_seq,
    };
    let fixture = prepare_runtime_fixture(
        runtime_directory,
        node_executable,
        compiled,
        &binding,
        opening_event_id,
    )?;
    binding.runtime_release_digest = fixture.release.payload_digest.clone();
    binding.runtime_build_digest = fixture.hello.runtime_build_digest.clone();
    let fixture = prepare_runtime_fixture(
        runtime_directory,
        node_executable,
        compiled,
        &binding,
        opening_event_id,
    )?;
    let profile = platform.pinned_runtime_profile(compiled);
    contract.validate_runtime_profile(&profile)?;
    let context = runtime_context(
        session_id,
        &runtime_binding_id,
        OperationId::from(new_id(if resume {
            "runtime-resume"
        } else {
            "runtime-create"
        })),
        compiled,
    );
    let open_command = if resume {
        RuntimeCommand::Resume(RuntimeResumeParams {
            context,
            compatibility_admission_input_digest: compatibility_digest
                .ok_or("resume requires compatibility digest")?,
            checkpoint: None,
        })
    } else {
        RuntimeCommand::Create(RuntimeCreateParams {
            context,
            profile_kind: nomifun_agent_contracts::RuntimeProfileKind::ManagedMinimal,
            full_auto: FullAutoExecutionWire::fixed(),
            initial_capabilities: BTreeSet::new(),
            on_demand_capabilities: BTreeSet::new(),
            typed_resource_bindings: Vec::new(),
        })
    };
    let process = RuntimeProcessConfig::pinned_app_server(
        node_executable,
        runtime_directory,
        &fixture.target_id,
        &fixture.release,
    )?;
    let managed = platform
        .runtime_port()
        .launch(RuntimeLaunchRequest {
            process,
            credential: InheritedHandleCredential::new(
                b"chat-minimal-runtime-credential".to_vec(),
            )?,
            release: fixture.release,
            hello_expectation: RuntimeHelloExpectation::from_payload(fixture.hello),
            profile,
            open_command,
            ingress: Arc::new(SessionRuntimeIngress {
                platform: Arc::clone(&platform),
                agent_session_id: session_id.clone(),
            }),
            client_limits: ClientLimits::default(),
            dispose_timeout: Duration::from_millis(500),
        })
        .await?;
    assert_eq!(managed.binding(), &binding);
    assert_eq!(
        platform.runtime_port().binding(&runtime_binding_id).await,
        Some(binding.clone())
    );
    Ok(OpenRuntime { binding })
}

async fn dispose_runtime(
    platform: &AgentPlatform,
    binding: &RuntimeBindingContract,
    reason: &str,
) -> TestResult<()> {
    let report = platform
        .runtime_port()
        .dispose(RuntimeSessionDisposeParams {
            agent_session_id: binding.agent_session_id.clone(),
            runtime_binding_id: binding.runtime_binding_id.clone(),
            operation_id: OperationId::from(new_id("runtime-dispose")),
            reason: CanonicalErrorCode::from(reason),
        })
        .await?;
    assert_eq!(report.rpc, DisposeRpcOutcome::Acked);
    Ok(())
}

fn prepare_runtime_fixture(
    directory: &Path,
    node_executable: &Path,
    compiled: &CompiledSnapshot,
    binding: &RuntimeBindingContract,
    opening_event_id: &EventId,
) -> TestResult<RuntimeFixture> {
    std::fs::create_dir_all(directory)?;
    std::fs::write(directory.join(RUNTIME_SCRIPT_FILE), RECORDED_RUNTIME_SCRIPT)?;
    let node_digest = sha256_path(node_executable)?;
    let script_digest = digest_bytes(RECORDED_RUNTIME_SCRIPT.as_bytes());
    let build_digest = digest_payload(&(node_digest.clone(), script_digest.clone()))?;
    let target_id = native_target_id().to_owned();
    let mut payload = RuntimeReleaseDescriptor::frozen_from_fixture()?.payload;
    let runtime_target = match payload.target_matrix.get_mut(&target_id) {
        Some(RuntimeReleaseTargetPayload::Required {
            runtime_target,
            sidecar_artifact,
            helper_artifacts,
            package_content_digest,
            ..
        }) => {
            sidecar_artifact.digest = node_digest;
            *helper_artifacts = vec![LogicalArtifactRef {
                artifact_id: ArtifactId::from("chat-minimal-recorded-app-server"),
                normalized_relative_path: "fixtures/chat-minimal/app-server".to_owned(),
                digest: script_digest,
            }];
            *package_content_digest = build_digest.clone();
            runtime_target.clone()
        }
        _ => return Err(format!("runtime release has no required target {target_id}").into()),
    };
    let release = RuntimeReleaseDescriptor::from_payload(payload)?;
    let expectation = release.hello_expectation(
        build_digest,
        runtime_target,
        compiled.content().required_runtime_features.clone(),
        BTreeSet::new(),
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
        idempotency_key: IdempotencyKey::from(new_id("runtime-bound")),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("runtime/bound".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId::from(
                binding.runtime_binding_id.as_ref().to_owned(),
            ),
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
            "runtime_bound_event": runtime_bound_event
        }))?,
    )?;
    Ok(RuntimeFixture {
        release,
        hello,
        target_id,
    })
}

fn hidden_initialization(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    profile: &nomifun_codex_runtime::PinnedRuntimeProfile,
    root: &Path,
) -> ChatMinimalHiddenInitialization {
    let launch = profile.launch_policy();
    ChatMinimalHiddenInitialization {
        capability_provider: registry.capabilities.len() as u64,
        skill_catalog: registry.skills.len() as u64,
        mcp: registry.mcp_tools.len() as u64,
        workspace: root.join("workspace").exists() as u64,
        agents_instructions: root.join("AGENTS.md").exists() as u64,
        git: root.join(".git").exists() as u64,
        shell: launch.builtin_coding_tools as u64,
        patch: launch.builtin_coding_tools as u64,
        memory: 0,
        knowledge: 0,
        business_context: 0,
        browser: 0,
        computer: 0,
        ssh: 0,
        office: 0,
        worker: 0,
        watcher: 0,
        resource_handle: profile.typed_resource_bindings.len() as u64,
        coding_context: [
            launch.codex_coding_base_instructions,
            launch.workspace_discovery,
            launch.agents_instructions,
            launch.tool_search,
            launch.code_mode,
            launch.review_workflow,
            launch.subagents,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count() as u64,
    }
}

fn user_principal(owner: &UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::now_v7())
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

fn sha256_path(path: &Path) -> Result<DigestHex, std::io::Error> {
    let mut file = File::open(path)?;
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

fn find_node() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NODE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in node_names() {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate.canonicalize().ok().or(Some(candidate));
            }
        }
    }
    None
}

fn node_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["node.exe", "node.cmd"]
    } else {
        &["node"]
    }
}

fn model_error(error: ChatModelError) -> std::io::Error {
    std::io::Error::other(format!("{:?}: {}", error.code, error.message))
}
