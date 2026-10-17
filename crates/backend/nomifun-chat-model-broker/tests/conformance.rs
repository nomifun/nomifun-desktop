use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{Stream, StreamExt, stream};
use nomifun_agent_contracts::{
    AgentSessionId, ChatRouteIdentity, ConnectionConfigRef, DigestHex, EventId, ModelRouteId,
    OperationId, ResolvedSnapshotId, ResolvedSnapshotRef, VersionString,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatBrokerPort, ChatCausality,
    ChatCapabilityObserver, ChatCausalityGate, ChatContentPart, ChatFinishReason, ChatMessage, ChatModelBroker,
    ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatModelFeature, ChatModelInput, ChatModelRequest,
    ChatModality, ChatProtocol, ChatProtocolAdapter, ChatResponseFormat, ChatRetryDirective,
    ChatRole, ChatRouteResolver, ChatRouteSelection, ChatToolChoice,
    CredentialLease, CredentialTarget, GeminiAdapter, OpenAiChatAdapter,
    OpenAiResponsesAdapter, PromptCachePolicy, ProviderCredentialRef,
    ProviderCredentialStore, ProviderIdRef, ProviderTransport, ProviderWireFrame,
    ProviderWireRequest, ProviderWireStream, ResponsesBridge,
    ResponsesBridgeEvent, ResponsesBridgeRequest, ResponsesInputContent, ResponsesInputItem,
    ResponsesRole, ResolvedChatRoute, ResolvedChatRouteSet, VertexAdapter,
    protocol_features, recorded_conformance_fixtures,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[test]
fn broker_reexports_the_exact_canonical_chat_types_and_wire_values() {
    use nomifun_agent_contracts::chat_model as canonical;

    let request = basic_request(&route(ChatProtocol::OpenaiChat, "shared-contract", 1));
    let expected = serde_json::to_value(&request).unwrap();
    // These assignments must compile without conversion: the old broker path
    // and the shared contract path name the same types, not matching copies.
    let canonical_request: canonical::ChatModelRequest = request;
    canonical_request.validate().unwrap();
    let broker_request: ChatModelRequest = canonical_request;
    assert_eq!(serde_json::to_value(&broker_request).unwrap(), expected);
    let decoded: canonical::ChatModelRequest = serde_json::from_value(expected).unwrap();
    assert_eq!(decoded, broker_request);

    let event: canonical::ChatModelEvent = ChatModelEvent::OutputTextDelta { text: "increment".into(), source: None };
    assert_eq!(serde_json::to_value(&event).unwrap(), serde_json::json!({
        "type": "output_text_delta", "text": "increment"
    }));
    let broker_event: ChatModelEvent = event;
    assert!(broker_event.is_semantic_output());
    assert!(!broker_event.is_terminal());
    let error: canonical::ChatModelError = ChatModelError::protocol_violation("invalid event");
    let broker_error: ChatModelError = error;
    assert_eq!(broker_error.retry, ChatRetryDirective::Never);
    assert_eq!(canonical::CHAT_MODEL_CONTRACT_VERSION, "chat-model-v1");
}

enum TransportScript {
    OpenError(ChatModelError),
    Frames(Vec<Result<ProviderWireFrame, ChatModelError>>),
    PendingOpen {
        started: Arc<Notify>,
        dropped: Arc<Notify>,
    },
    PendingStream {
        started: Arc<Notify>,
        dropped: Arc<Notify>,
    },
}

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

struct ScriptedTransport {
    calls: AtomicUsize,
    scripts: Mutex<VecDeque<TransportScript>>,
}

impl ScriptedTransport {
    fn new(scripts: impl IntoIterator<Item = TransportScript>) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            scripts: Mutex::new(scripts.into_iter().collect()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ProviderTransport for ScriptedTransport {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        assert_eq!(credential.credential_ref(), &request.credential_ref);
        assert_eq!(credential.target().model_route_id, request.route_identity.route_id);
        assert_eq!(
            credential.target().model_route_revision,
            request.route_identity.route_revision
        );
        assert_eq!(credential.target().provider_id, request.provider_id);
        assert_eq!(credential.target().protocol, request.protocol);
        assert_eq!(
            credential.target().connection_config_ref,
            request.connection_config_ref
        );
        assert_eq!(
            credential.target().config_revision_digest,
            request.config_revision_digest
        );
        self.calls.fetch_add(1, Ordering::AcqRel);
        let script = self
            .scripts
            .lock()
            .expect("scripted transport lock")
            .pop_front()
            .unwrap_or_else(|| {
                TransportScript::OpenError(ChatModelError::provider_unavailable(
                    "scripted transport exhausted",
                ))
            });
        match script {
            TransportScript::OpenError(error) => Err(error),
            TransportScript::Frames(frames) => Ok(Box::pin(stream::iter(frames))),
            TransportScript::PendingOpen { started, dropped } => {
                let _guard = NotifyOnDrop(dropped);
                started.notify_one();
                std::future::pending().await
            }
            TransportScript::PendingStream { started, dropped } => {
                Ok(Box::pin(stream::once(async move {
                    let _guard = NotifyOnDrop(dropped);
                    started.notify_one();
                    std::future::pending::<Result<ProviderWireFrame, ChatModelError>>().await
                })))
            }
        }
    }
}

struct StaticCausalityGate {
    calls: AtomicUsize,
    error: Option<ChatModelError>,
}

impl StaticCausalityGate {
    fn allow() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            error: None,
        })
    }

    fn reject(error: ChatModelError) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            error: Some(error),
        })
    }
}

#[async_trait]
impl ChatCausalityGate for StaticCausalityGate {
    async fn authorize(&self, _causality: &ChatCausality) -> Result<(), ChatModelError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

struct StaticRouteResolver {
    routes: ResolvedChatRouteSet,
}

#[async_trait]
impl ChatRouteResolver for StaticRouteResolver {
    async fn resolve(
        &self,
        _selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatModelError> {
        Ok(self.routes.clone())
    }
}

struct StaticCredentialStore {
    mismatch: bool,
}

struct OutputBoundRouteResolver {
    routes: ResolvedChatRouteSet,
    limits: BTreeMap<String, u32>,
}

#[async_trait]
impl ChatRouteResolver for OutputBoundRouteResolver {
    async fn resolve(&self, _selection: &ChatRouteSelection) -> Result<ResolvedChatRouteSet, ChatModelError> {
        Ok(self.routes.clone())
    }
    async fn output_limit_for_route(&self, route: &ResolvedChatRoute) -> Result<Option<u32>, ChatModelError> {
        Ok(self.limits.get(route.model_route_id.as_ref()).copied())
    }
}

struct OutputBoundCaptureTransport {
    requests: Arc<Mutex<Vec<ProviderWireRequest>>>,
}

#[async_trait]
impl ProviderTransport for OutputBoundCaptureTransport {
    async fn open_stream(&self, request: ProviderWireRequest, _credential: CredentialLease) -> Result<ProviderWireStream, ChatModelError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        if requests.len() == 1 {
            return Err(ChatModelError::new(ChatModelErrorCode::ProviderUnavailable,
                "fixture selects the next configured route", ChatRetryDirective::Failover));
        }
        Ok(Box::pin(stream::iter(successful_frames("output-bound", "bounded reply"))))
    }
}

#[derive(Default)]
struct RecordingCapabilityObserver {
    observations: Mutex<Vec<(String, ChatModelFeature)>>,
}

#[async_trait]
impl ChatCapabilityObserver for RecordingCapabilityObserver {
    async fn record_unsupported(
        &self,
        route: &ResolvedChatRoute,
        feature: ChatModelFeature,
    ) {
        self.observations
            .lock()
            .unwrap()
            .push((route.model_route_id.as_ref().to_owned(), feature));
    }
}

#[async_trait]
impl ProviderCredentialStore for StaticCredentialStore {
    async fn lease(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
    ) -> Result<CredentialLease, ChatModelError> {
        let target = if self.mismatch {
            CredentialTarget {
                provider_id: ProviderIdRef("different-provider".to_owned()),
                ..target.clone()
            }
        } else {
            target.clone()
        };
        Ok(CredentialLease::new(
            credential_ref.clone(),
            target,
            "credential-handle-recorded",
        ))
    }
}

fn route(protocol: ChatProtocol, id: &str, revision: u64) -> ResolvedChatRoute {
    ResolvedChatRoute {
        model_route_id: ModelRouteId(id.to_owned()),
        model_route_revision: revision,
        provider_id: ProviderIdRef(format!("provider-{id}")),
        model: format!("model-{id}"),
        protocol,
        connection_config_ref: ConnectionConfigRef(format!("connection-{id}")),
        config_revision_digest: DigestHex("a".repeat(64)),
        credential_ref: ProviderCredentialRef(format!("credential-ref-{id}")),
        features: protocol_features(protocol),
        activation_features: BTreeSet::new(),
    }
}

fn basic_request(route: &ResolvedChatRoute) -> ChatModelRequest {
    let route_identity = ChatRouteIdentity::new(
        "fixture@1",
        nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
        route.model_route_id.clone(),
        route.model_route_revision,
    );
    ChatModelRequest {
        contract_version: VersionString("chat-model-v1".to_owned()),
        causality: ChatCausality {
            agent_session_id: AgentSessionId(
                "0190f5fe-7c00-7a00-8000-000000009001".to_owned(),
            ),
            turn_operation_id: OperationId("turn-operation-test".to_owned()),
            causation_event_id: EventId("causation-event-test".to_owned()),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId("snapshot-test".to_owned()),
                snapshot_digest: DigestHex("9".repeat(64)),
            },
            route_identity: route_identity.clone(),
            operation_id: OperationId("model-operation-test".to_owned()),
        },
        route: route_identity,
        input: ChatModelInput {
            instructions: vec!["Answer directly.".to_owned()],
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: vec![ChatContentPart::Text {
                    text: "Hello".to_owned(),
                }],
                provider_round_id: None,
            }],
            tools: Vec::new(),
            tool_choice: ChatToolChoice::None,
            parallel_tool_calls: None,
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

fn frame(event: &str, data: serde_json::Value) -> Result<ProviderWireFrame, ChatModelError> {
    Ok(ProviderWireFrame {
        diagnostic: None,
        event: event.to_owned(),
        data,
    })
}

fn successful_frames(response_id: &str, text: &str) -> Vec<Result<ProviderWireFrame, ChatModelError>> {
    vec![
        frame("response.start", serde_json::json!({"id": response_id})),
        frame("text.delta", serde_json::json!({"text": text})),
        frame(
            "usage",
            serde_json::json!({"input_tokens": 4, "output_tokens": 2}),
        ),
        frame("done", serde_json::json!({"finish_reason": "stop"})),
    ]
}

fn transport_map(
    overrides: impl IntoIterator<Item = (ChatProtocol, Arc<dyn ProviderTransport>)>,
) -> BTreeMap<ChatProtocol, Arc<dyn ProviderTransport>> {
    let fallback = ScriptedTransport::new([TransportScript::OpenError(
        ChatModelError::provider_unavailable("unexpected adapter invocation"),
    )]);
    let mut transports = ChatProtocol::ALL
        .into_iter()
        .map(|protocol| {
            (
                protocol,
                provider_transport(&fallback),
            )
        })
        .collect::<BTreeMap<_, _>>();
    transports.extend(overrides);
    transports
}

fn provider_transport<T>(transport: &Arc<T>) -> Arc<dyn ProviderTransport>
where
    T: ProviderTransport + 'static,
{
    transport.clone()
}

fn adapters(
    transports: &BTreeMap<ChatProtocol, Arc<dyn ProviderTransport>>,
) -> Vec<Arc<dyn ChatProtocolAdapter>> {
    vec![
        Arc::new(AnthropicAdapter::new(
            transports[&ChatProtocol::Anthropic].clone(),
        )),
        Arc::new(OpenAiChatAdapter::new(
            transports[&ChatProtocol::OpenaiChat].clone(),
        )),
        Arc::new(OpenAiResponsesAdapter::new(
            transports[&ChatProtocol::OpenaiResponses].clone(),
        )),
        Arc::new(GeminiAdapter::new(
            transports[&ChatProtocol::Gemini].clone(),
        )),
        Arc::new(BedrockAdapter::new(
            transports[&ChatProtocol::Bedrock].clone(),
        )),
        Arc::new(VertexAdapter::new(
            transports[&ChatProtocol::Vertex].clone(),
        )),
    ]
}

fn broker(
    gate: Arc<dyn ChatCausalityGate>,
    routes: ResolvedChatRouteSet,
    store: Arc<dyn ProviderCredentialStore>,
    transports: &BTreeMap<ChatProtocol, Arc<dyn ProviderTransport>>,
    retry_policy: BrokerRetryPolicy,
) -> Arc<ChatModelBroker> {
    Arc::new(
        ChatModelBroker::new(
            gate,
            Arc::new(StaticRouteResolver { routes }),
            store,
            adapters(transports),
            retry_policy,
        )
        .expect("valid six-protocol broker"),
    )
}

fn broker_with_observer(
    gate: Arc<dyn ChatCausalityGate>,
    routes: ResolvedChatRouteSet,
    store: Arc<dyn ProviderCredentialStore>,
    transports: &BTreeMap<ChatProtocol, Arc<dyn ProviderTransport>>,
    retry_policy: BrokerRetryPolicy,
    observer: Arc<dyn ChatCapabilityObserver>,
) -> Arc<ChatModelBroker> {
    Arc::new(
        ChatModelBroker::new_with_capability_observer(
            gate,
            Arc::new(StaticRouteResolver { routes }),
            store,
            adapters(transports),
            retry_policy,
            observer,
        )
        .expect("valid six-protocol broker"),
    )
}

#[test]
fn recorded_wire_fixtures_cover_the_exact_six_protocols() {
    let fixtures = recorded_conformance_fixtures();
    let protocols = fixtures
        .iter()
        .map(|fixture| fixture.protocol)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        protocols,
        ChatProtocol::ALL.into_iter().collect::<BTreeSet<_>>()
    );
    assert!(fixtures.iter().all(|fixture| fixture.validate().is_ok()));
    assert!(fixtures.iter().any(|fixture| fixture.coverage.image_input));
    assert!(fixtures.iter().any(|fixture| fixture.coverage.audio_input));
    assert!(
        fixtures
            .iter()
            .any(|fixture| fixture.coverage.native_responses_items)
    );
}

#[test]
fn every_recorded_wire_decodes_to_its_canonical_event_sequence() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapters = adapters(&transports)
        .into_iter()
        .map(|adapter| (adapter.protocol(), adapter))
        .collect::<BTreeMap<_, _>>();

    for fixture in recorded_conformance_fixtures() {
        assert!(fixture.wire_events.iter().all(|frame| frame.diagnostic.is_none()));
        let adapter = &adapters[&fixture.protocol];
        let decoded = fixture
            .wire_events
            .clone()
            .into_iter()
            .flat_map(|frame| adapter.decode_frame(frame).expect("recorded frame"))
            .collect::<Vec<_>>();
        assert_eq!(decoded, fixture.expected_events, "{}", fixture.scenario_id);
    }
}

#[test]
fn official_decoders_preserve_redacted_http_context_on_verified_errors() {
    use nomifun_agent_contracts::{ModelFailureDiagnostic, ModelFailureReason};
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    for adapter in adapters(&transports) {
        let protocol = adapter.protocol();
        let (event, data) = match protocol {
            ChatProtocol::Bedrock => ("bedrock.exception", serde_json::json!({
                "error": {"code": "accessDeniedException"}
            })),
            ChatProtocol::Gemini => ("json", serde_json::json!({
                "error": {"code": 403, "status": "PERMISSION_DENIED", "message": "private-native-key"}
            })),
            _ => ("error", serde_json::json!({"error": {"type": "permission_error",
                "message": "private-native-key", "request_id": "private-native-key"}})),
        };
        let mut context = ModelFailureDiagnostic::new(ModelFailureReason::AuthFailed);
        context.http_status = Some(403);
        context.request_id = Some("req_redacted_context".to_owned());
        context.endpoint = Some("https://provider.example.test/v1".to_owned());
        let frame = ProviderWireFrame { event: event.to_owned(), data, diagnostic: Some(context.clone()) };
        let request = basic_request(&route(protocol, "error-context", 1));
        let mut decoder = adapter.new_frame_decoder_for(&request).unwrap();
        for error in [adapter.decode_frame(frame.clone()).unwrap_err(), decoder.decode_frame(frame).unwrap_err()] {
            let mut expected = context.clone();
            expected.reason = ModelFailureReason::PermissionDenied;
            assert_eq!(error.diagnostic, Some(expected), "{protocol:?}");
            assert!(!serde_json::to_string(&error).unwrap().contains("private-native-key"));
        }
    }
}

#[test]
fn bounded_json_fallback_decodes_openai_chat_and_gemini_responses() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapters = adapters(&transports)
        .into_iter()
        .map(|adapter| (adapter.protocol(), adapter))
        .collect::<BTreeMap<_, _>>();
    for (protocol, data) in [
        (
            ChatProtocol::OpenaiChat,
            serde_json::json!({
                "id": "chatcmpl_1",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hello"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
            }),
        ),
        (
            ChatProtocol::Gemini,
            serde_json::json!({
                "responseId": "gemini_1",
                "candidates": [{
                    "content": {"role": "model", "parts": [{"text": "hello"}]},
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"promptTokenCount": 2, "candidatesTokenCount": 1, "totalTokenCount": 3}
            }),
        ),
    ] {
        let route = route(protocol, "json-fallback", 1);
        let request = basic_request(&route);
        let mut decoder = adapters[&protocol]
            .new_frame_decoder_for(&request)
            .expect("official adapter has an attempt-local decoder");
        let events = decoder
            .decode_frame(ProviderWireFrame {
                diagnostic: None,
                event: "json".into(),
                data,
            })
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ChatModelEvent::OutputTextDelta { text, .. } if text == "hello"
        )));
        assert!(matches!(
            events.last(),
            Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })
        ));
    }
}

#[test]
fn openai_chat_raw_sse_shape_decodes_text_tools_usage_and_finish() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = OpenAiChatAdapter::new(transports[&ChatProtocol::OpenaiChat].clone());
    let frames = [
        serde_json::json!({
            "id": "chatcmpl_raw_1",
            "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
        }),
        serde_json::json!({
            "id": "chatcmpl_raw_1",
            "choices": [{"index": 0, "delta": {"content": "Hello "}, "finish_reason": null}]
        }),
        serde_json::json!({
            "id": "chatcmpl_raw_1",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_raw_1",
                        "type": "function",
                        "function": {"name": "lookup", "arguments": "{\"q\":\""}
                    }]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl_raw_1",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "raw\"}"}
                    }]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl_raw_1",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 7,
                "completion_tokens_details": {"reasoning_tokens": 2},
                "prompt_tokens_details": {"cached_tokens": 3}
            }
        }),
    ];
    let events = frames
        .into_iter()
        .flat_map(|data| {
            adapter
                .decode_frame(ProviderWireFrame {
                    diagnostic: None,
                    event: "message".to_owned(),
                    data,
                })
                .expect("OpenAI raw frame")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![
            ChatModelEvent::ResponseStarted {
                provider_response_id: Some(nomifun_chat_model_broker::ProviderResponseId(
                    "chatcmpl_raw_1".to_owned()
                ))
            },
            ChatModelEvent::OutputTextDelta {
                text: "Hello ".to_owned(),
                source: None,
            },
            ChatModelEvent::ToolCallDelta {
                call_id: nomifun_chat_model_broker::ToolCallId("call_raw_1".to_owned()),
                name: "lookup".to_owned(),
                arguments_delta: "{\"q\":\"".to_owned()
            },
            ChatModelEvent::ToolCallDelta {
                call_id: nomifun_chat_model_broker::ToolCallId("call_raw_1".to_owned()),
                name: "lookup".to_owned(),
                arguments_delta: "raw\"}".to_owned()
            },
            ChatModelEvent::ToolCallCompleted {
                call: nomifun_chat_model_broker::ChatToolCall {
                    call_id: nomifun_chat_model_broker::ToolCallId("call_raw_1".to_owned()),
                    name: "lookup".to_owned(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(
                        serde_json::json!({"q": "raw"})
                    ),
                    provider_metadata: None,
                }
            },
            ChatModelEvent::Usage {
                usage: nomifun_chat_model_broker::ChatUsage {
                    input_tokens: 12,
                    output_tokens: 7,
                    reasoning_tokens: 2,
                    cache_write_tokens: 0,
                    cache_read_tokens: 3,
                    audio_input_tokens: 0,
                    audio_output_tokens: 0,
                    provider_reported: BTreeMap::new(),
                }
            },
            ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::ToolCalls
            }
        ]
    );
}

#[test]
fn chat_content_and_reasoning_in_one_frame_share_the_response_text_source() {
    let transports = transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = OpenAiChatAdapter::new(transports[&ChatProtocol::OpenaiChat].clone());
    let mut events = Vec::new();
    for delta in [serde_json::json!({"content":"CL", "reasoning_content":"check"}),
        serde_json::json!({"content":"I 和 SKILL 都已安装成功。"})]
    {
        events.extend(adapter.decode_frame(ProviderWireFrame { diagnostic:None, event:"message".into(),
            data:serde_json::json!({"id":"stepfun_response", "choices":[{"index":0, "delta":delta}]})
        }).unwrap());
    }
    assert!(events.iter().any(|event| matches!(event, ChatModelEvent::ReasoningDelta {text} if text == "check")));
    let text = events.iter().filter_map(|event| match event {
        ChatModelEvent::OutputTextDelta { text, source:None } => Some(text.as_str()), _ => None,
    }).collect::<String>();
    assert_eq!(text, "CLI 和 SKILL 都已安装成功。");
}

#[test]
fn openai_chat_id_less_prelude_and_trailing_chunks_join_the_identified_stream() {
    let mut decoder =
        nomifun_chat_model_broker::new_verification_frame_decoder(ChatProtocol::OpenaiChat);
    let frames = [
        serde_json::json!({
            "id": "", "choices": [], "created": 0, "model": "gpt-6-sol", "object": ""
        }),
        serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "gpt-6-sol",
            "obfuscation": "x",
            "service_tier": "default",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "nomifun_protocol_echo", "arguments": ""}
                    }]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl-1",
            "choices": [{
                "index": 0,
                "delta": {
                    "content": "",
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "{\"value\":\"ok\"}"}
                    }]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl-1",
            "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": "tool_calls"}]
        }),
        serde_json::json!({
            "id": "",
            "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": null}]
        }),
        serde_json::json!({
            "id": "chatcmpl-1",
            "choices": [],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }),
    ];
    let events = frames
        .into_iter()
        .flat_map(|data| {
            decoder
                .decode_frame(ProviderWireFrame {
                    diagnostic: None,
                    event: "message".to_owned(),
                    data,
                })
                .expect("Azure-style Chat frame")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ChatModelEvent::ResponseStarted { .. }))
            .count(),
        1,
        "events: {events:?}"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ChatModelEvent::ToolCallCompleted { call }
            if call.name == "nomifun_protocol_echo"
                && call.arguments.0 == serde_json::json!({"value": "ok"})
    )));
    assert!(matches!(
        events.last(),
        Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })
    ));
}

#[test]
fn openai_chat_id_less_prelude_text_stream_completes_once() {
    let mut decoder =
        nomifun_chat_model_broker::new_verification_frame_decoder(ChatProtocol::OpenaiChat);
    let frames = [
        serde_json::json!({
            "id": "", "choices": [], "created": 0, "model": "gpt-6-sol", "object": ""
        }),
        serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant", "content": ""},
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{"index": 0, "delta": {"content": "OK"}, "finish_reason": null}]
        }),
        serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": "stop"}]
        }),
        serde_json::json!({
            "id": "",
            "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": null}]
        }),
        serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }),
    ];
    let events = frames
        .into_iter()
        .flat_map(|data| {
            decoder
                .decode_frame(ProviderWireFrame {
                    diagnostic: None,
                    event: "message".to_owned(),
                    data,
                })
                .expect("Azure-style Chat frame")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ChatModelEvent::ResponseStarted { .. }))
            .count(),
        1,
        "events: {events:?}"
    );
    let text = events
        .iter()
        .filter_map(|event| match event {
            ChatModelEvent::OutputTextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "OK");
    assert!(matches!(
        events.last(),
        Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })
    ));
}

#[test]
fn openai_chat_contradicting_identified_ids_remain_a_stream_violation() {
    let mut decoder =
        nomifun_chat_model_broker::new_verification_frame_decoder(ChatProtocol::OpenaiChat);
    let first = decoder
        .decode_frame(ProviderWireFrame {
            diagnostic: None,
            event: "message".to_owned(),
            data: serde_json::json!({
                "id": "chatcmpl-a",
                "choices": [{"index": 0, "delta": {"content": "x"}, "finish_reason": null}]
            }),
        })
        .expect("first identified frame decodes");
    assert!(first
        .iter()
        .any(|event| matches!(event, ChatModelEvent::ResponseStarted { .. })));
    let error = decoder
        .decode_frame(ProviderWireFrame {
            diagnostic: None,
            event: "message".to_owned(),
            data: serde_json::json!({
                "id": "chatcmpl-b",
                "choices": [{"index": 0, "delta": {"content": "y"}, "finish_reason": null}]
            }),
        })
        .expect_err("a second identified response violates the event sequence");
    assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
}

#[test]
fn openai_chat_done_marker_completes_a_stream_without_finish_reason() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = OpenAiChatAdapter::new(transports[&ChatProtocol::OpenaiChat].clone());
    let frames = [
        serde_json::json!({
            "id": "chatcmpl_done_only",
            "choices": [{
                "index": 0,
                "delta": {"content": "complete me"},
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl_done_only",
            "choices": [],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2
            }
        }),
        serde_json::json!({}),
    ];
    let events = frames
        .into_iter()
        .enumerate()
        .flat_map(|(index, data)| {
            adapter
                .decode_frame(ProviderWireFrame {
                    diagnostic: None,
                    event: if index == 2 {
                        "done".to_owned()
                    } else {
                        "message".to_owned()
                    },
                    data,
                })
                .expect("OpenAI raw frame")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![
            ChatModelEvent::ResponseStarted {
                provider_response_id: Some(nomifun_chat_model_broker::ProviderResponseId(
                    "chatcmpl_done_only".to_owned()
                ))
            },
            ChatModelEvent::OutputTextDelta {
                text: "complete me".to_owned(),
                source: None,
            },
            ChatModelEvent::Usage {
                usage: nomifun_chat_model_broker::ChatUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                    reasoning_tokens: 0,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    audio_input_tokens: 0,
                    audio_output_tokens: 0,
                    provider_reported: BTreeMap::new(),
                }
            },
            ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ]
    );
}

#[test]
fn gemini_raw_sse_and_json_shapes_decode_parts_usage_and_finish() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = GeminiAdapter::new(transports[&ChatProtocol::Gemini].clone());
    let frames = [
        serde_json::json!({
            "responseId": "gemini-raw-1",
            "candidates": [{
                "index": 0,
                "content": {
                    "role": "model",
                    "parts": [{"text": "thinking", "thought": true}]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 4,
                "thoughtsTokenCount": 1,
                "cachedContentTokenCount": 6
            }
        }),
        serde_json::json!({
            "candidates": [],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 5,
                "thoughtsTokenCount": 1,
                "cachedContentTokenCount": 6
            }
        }),
        serde_json::json!({
            "candidates": [{
                "index": 0,
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "id": "gemini-call-1",
                            "name": "lookup",
                            "args": {"q": "raw"}
                        },
                        "thoughtSignature": "sig-1"
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 8,
                "thoughtsTokenCount": 2,
                "cachedContentTokenCount": 6
            }
        }),
    ];
    let events = frames
        .into_iter()
        .flat_map(|data| {
            adapter
                .decode_frame(ProviderWireFrame {
                    diagnostic: None,
                    event: "message".to_owned(),
                    data,
                })
                .expect("Gemini raw frame")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![
            ChatModelEvent::ResponseStarted {
                provider_response_id: Some(nomifun_chat_model_broker::ProviderResponseId(
                    "gemini-raw-1".to_owned()
                ))
            },
            ChatModelEvent::ReasoningDelta {
                text: "thinking".to_owned()
            },
            ChatModelEvent::ToolCallCompleted {
                call: nomifun_chat_model_broker::ChatToolCall {
                    call_id: nomifun_chat_model_broker::ToolCallId("gemini-call-1".to_owned()),
                    name: "lookup".to_owned(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(
                        serde_json::json!({"q": "raw"})
                    ),
                    provider_metadata: Some(nomifun_agent_contracts::StrictJsonValue(
                        serde_json::json!({"thoughtSignature": "sig-1"})
                    )),
                }
            },
            ChatModelEvent::Usage {
                usage: nomifun_chat_model_broker::ChatUsage {
                    input_tokens: 20,
                    output_tokens: 8,
                    reasoning_tokens: 2,
                    cache_write_tokens: 0,
                    cache_read_tokens: 6,
                    audio_input_tokens: 0,
                    audio_output_tokens: 0,
                    provider_reported: BTreeMap::new(),
                }
            },
            ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::ToolCalls
            }
        ]
    );

    let json_events = adapter
        .decode_frame(ProviderWireFrame {
            diagnostic: None,
            event: "json".to_owned(),
            data: serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": "done"}]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 1,
                    "candidatesTokenCount": 1
                }
            }),
        })
        .expect("Gemini JSON frame");
    assert_eq!(
        json_events,
        vec![
            ChatModelEvent::OutputTextDelta {
                text: "done".to_owned(),
                source: None,
            },
            ChatModelEvent::Usage {
                usage: nomifun_chat_model_broker::ChatUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    reasoning_tokens: 0,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    audio_input_tokens: 0,
                    audio_output_tokens: 0,
                    provider_reported: BTreeMap::new(),
                }
            },
            ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ]
    );
}

#[test]
fn gemini_blocked_payload_is_provider_failure_before_empty_candidate_validation() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = GeminiAdapter::new(transports[&ChatProtocol::Gemini].clone());
    let error = adapter
        .decode_frame(ProviderWireFrame {
            diagnostic: None,
            event: "message".to_owned(),
            data: serde_json::json!({
                "candidates": [],
                "promptFeedback": {"blockReason": "SAFETY"}
            }),
        })
        .expect_err("blocked Gemini response must not be treated as malformed JSON");
    assert_eq!(error.code, ChatModelErrorCode::ProviderUnavailable);
}

#[test]
fn gemini_tool_result_continuation_uses_function_name_and_preserves_call_id() {
    let fixture = recorded_conformance_fixtures()
        .into_iter()
        .find(|fixture| fixture.protocol == ChatProtocol::Gemini)
        .expect("Gemini fixture");
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = GeminiAdapter::new(transports[&ChatProtocol::Gemini].clone());
    let lease = CredentialLease::new(
        fixture.route.credential_ref.clone(),
        CredentialTarget::for_route(&fixture.route),
        "opaque-fixture-handle",
    );
    let body = adapter
        .encode_request(&fixture.request, &fixture.route, &lease)
        .expect("Gemini continuation must encode")
        .body;
    let parts = body["contents"]
        .as_array()
        .expect("Gemini contents")
        .iter()
        .flat_map(|content| {
            content["parts"]
                .as_array()
                .expect("Gemini content parts")
        })
        .collect::<Vec<_>>();
    let function_call = parts
        .iter()
        .find_map(|part| part.get("functionCall"))
        .expect("Gemini functionCall");
    let function_response = parts
        .iter()
        .find_map(|part| part.get("functionResponse"))
        .expect("Gemini functionResponse");

    assert_eq!(function_call["id"], "call-history-gemini");
    assert_eq!(function_call["name"], "lookup");
    assert_eq!(function_response["id"], "call-history-gemini");
    assert_eq!(function_response["name"], "lookup");
    assert_ne!(function_response["name"], function_response["id"]);
}

async fn bound_gemini_tool_call(
    route: &ResolvedChatRoute,
    signature_field: &str,
) -> nomifun_chat_model_broker::ChatToolCall {
    bound_gemini_tool_call_from_attempt(route, signature_field, None).await
}

async fn bound_gemini_tool_call_from_attempt(
    route: &ResolvedChatRoute,
    signature_field: &str,
    primary: Option<ResolvedChatRoute>,
) -> nomifun_chat_model_broker::ChatToolCall {
    let mut part = serde_json::json!({
            "functionCall": {"id": "signed-call", "name": "lookup", "args": {"q": "fixture"}}
    });
    part[signature_field] = serde_json::json!("opaque-tool-signature");
    let frames = [
            serde_json::json!({
                "responseId": "signed-tool-response",
                "candidates": [{"content": {"role": "model", "parts": [part]}}]
            }),
            serde_json::json!({
                "responseId": "signed-tool-response",
                "candidates": [{"content": {"role": "model", "parts": []}, "finishReason": "STOP"}]
            }),
    ];
    let transport = ScriptedTransport::new([TransportScript::Frames(
        frames.into_iter().map(|data| frame("message", data)).collect(),
    )]);
    let primary_transport = ScriptedTransport::new([TransportScript::OpenError(ChatModelError::new(
        ChatModelErrorCode::ProviderUnavailable, "fixture primary failure", ChatRetryDirective::Failover,
    ))]);
    let transports = transport_map([
        (ChatProtocol::Gemini, provider_transport(&transport)),
        (ChatProtocol::OpenaiChat, provider_transport(&primary_transport)),
    ]);
    let routes = if let Some(primary) = primary {
        ResolvedChatRouteSet { primary, failovers: vec![route.clone()] }
    } else {
        ResolvedChatRouteSet { primary: route.clone(), failovers: vec![] }
    };
    let request = basic_request(&routes.primary);
    let broker = broker(
        StaticCausalityGate::allow(), routes,
        Arc::new(StaticCredentialStore { mismatch: false }), &transports,
        BrokerRetryPolicy { max_total_attempts: 2, max_attempts_per_route: 1 },
    );
    let events = broker.open_stream(request).await.unwrap()
        .map(|result| result.unwrap().event).collect::<Vec<_>>().await;
    assert!(!events.iter().any(|event| matches!(event,
            ChatModelEvent::ReasoningDelta { .. } | ChatModelEvent::ReasoningSignature { .. })));
    assert!(matches!(events.last(), Some(ChatModelEvent::Completed {
            finish_reason: ChatFinishReason::ToolCalls
        })));
    let calls = events.into_iter().filter_map(|event| match event {
            ChatModelEvent::ToolCallCompleted { call } => Some(call),
            _ => None,
        }).collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let call: nomifun_chat_model_broker::ChatToolCall =
        serde_json::from_value(serde_json::to_value(&calls[0]).unwrap()).unwrap();
    assert_eq!(call.provider_metadata.as_ref().unwrap().0["_nomifun_producing_route_digest"],
        serde_json::to_value(nomifun_agent_contracts::digest_payload(route).unwrap()).unwrap());
    call
}

fn gemini_tool_continuation(
    route: &ResolvedChatRoute,
    call: nomifun_chat_model_broker::ChatToolCall,
) -> ChatModelRequest {
    let mut request = basic_request(route);
    request.input.messages.push(ChatMessage {
            role: ChatRole::Assistant, provider_round_id: None,
            content: vec![ChatContentPart::ToolCall {
                call_id: call.call_id.clone(), name: call.name, arguments: call.arguments,
                provider_metadata: call.provider_metadata,
            }],
    });
    request.input.messages.push(ChatMessage {
            role: ChatRole::Tool, provider_round_id: None,
            content: vec![ChatContentPart::ToolResult {
                call_id: call.call_id,
                output: vec![nomifun_chat_model_broker::ChatToolResultPart::Text { text: "found".into() }],
                is_error: false,
            }],
    });
    request
}

#[tokio::test]
async fn gemini_signed_tool_sse_round_trips_to_the_next_request_without_reasoning_text() {
    let adapter = GeminiAdapter::new(ScriptedTransport::new([]));
    let route = route(ChatProtocol::Gemini, "signed-tool", 1);
    let lease = CredentialLease::new(
        route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle",
    );
    for signature_field in ["thoughtSignature", "thought_signature"] {
        let call = bound_gemini_tool_call(&route, signature_field).await;
        let request = gemini_tool_continuation(&route, call);
        let body = adapter.encode_request(&request, &route, &lease).unwrap().body;
        assert_eq!(body["contents"][1]["parts"][0], serde_json::json!({
            "functionCall": {"id": "signed-call", "name": "lookup", "args": {"q": "fixture"}},
            "thoughtSignature": "opaque-tool-signature"
        }));
        assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["id"], "signed-call");
        assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["name"], "lookup");
    }
}

#[tokio::test]
async fn gemini_signed_tools_reject_unknown_origin_and_every_changed_route_identity() {
    let original = route(ChatProtocol::Gemini, "signed-route-fence", 1);
    let call = bound_gemini_tool_call(&original, "thoughtSignature").await;
    let request = gemini_tool_continuation(&original, call);
    let transports = transport_map([]);
    let adapters = adapters(&transports);
    let mut changed = Vec::new();
    let mut route = original.clone(); route.model = "another-model".into(); changed.push(route);
    let mut route = original.clone(); route.provider_id = ProviderIdRef("another-provider".into()); changed.push(route);
    let mut route = original.clone(); route.model_route_id = ModelRouteId("another-route".into()); changed.push(route);
    let mut route = original.clone(); route.model_route_revision += 1; changed.push(route);
    let mut route = original.clone(); route.connection_config_ref = ConnectionConfigRef("another-connection".into()); changed.push(route);
    let mut route = original.clone(); route.config_revision_digest = DigestHex("b".repeat(64)); changed.push(route);
    let mut route = original.clone(); route.credential_ref = ProviderCredentialRef("another-credential-ref".into()); changed.push(route);
    for protocol in ChatProtocol::ALL.into_iter().filter(|protocol| *protocol != ChatProtocol::Gemini) {
        let mut route = original.clone(); route.protocol = protocol; route.features = protocol_features(protocol); changed.push(route);
    }
    for route in changed {
        let adapter = adapters.iter().find(|adapter| adapter.protocol() == route.protocol).unwrap();
        let lease = CredentialLease::new(
            route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle",
        );
        let error = adapter.encode_request(&request, &route, &lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature, "{route:?}");
        assert_eq!(error.retry, ChatRetryDirective::Never);
        // An explicit new selection with the old signed history must fail
        // before claiming causality or obtaining transport credentials.
        let gate = StaticCausalityGate::allow();
        let mut switched = request.clone();
        let selected = basic_request(&route);
        switched.route = selected.route;
        switched.causality = selected.causality;
        let broker = broker(
            gate.clone(), ResolvedChatRouteSet { primary: route, failovers: vec![] },
            Arc::new(StaticCredentialStore { mismatch: false }), &transports,
            BrokerRetryPolicy { max_total_attempts: 1, max_attempts_per_route: 1 },
        );
        let error = match broker.open_stream(switched).await {
            Err(error) => error,
            Ok(_) => panic!("changed route accepted signed history"),
        };
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.retry, ChatRetryDirective::Never);
        assert_eq!(gate.calls.load(Ordering::Acquire), 0);
    }
    let adapter = adapters.iter().find(|adapter| adapter.protocol() == ChatProtocol::Gemini).unwrap();
    let lease = CredentialLease::new(
        original.credential_ref.clone(), CredentialTarget::for_route(&original), "fixture-handle",
    );
    for origin in [None, Some(serde_json::json!("unknown-origin")), Some(serde_json::json!(42))] {
        let mut request = request.clone();
        let part = request.input.messages.iter_mut().flat_map(|message| &mut message.content)
            .find(|part| matches!(part, ChatContentPart::ToolCall { .. })).unwrap();
        if let ChatContentPart::ToolCall { provider_metadata: Some(metadata), .. } = part {
            let metadata = metadata.0.as_object_mut().unwrap();
            metadata.remove("_nomifun_producing_route_digest");
            if let Some(origin) = origin { metadata.insert("_nomifun_producing_route_digest".into(), origin); }
        }
        let error = adapter.encode_request(&request, &original, &lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.retry, ChatRetryDirective::Never);
    }
}

#[tokio::test]
async fn gemini_tool_signature_origin_uses_the_actual_failover_attempt() {
    let producing = route(ChatProtocol::Gemini, "producing-failover", 1);
    let primary = route(ChatProtocol::OpenaiChat, "failed-primary", 1);
    let call = bound_gemini_tool_call_from_attempt(&producing, "thoughtSignature", Some(primary.clone())).await;
    let request = gemini_tool_continuation(&producing, call);
    let adapter = GeminiAdapter::new(ScriptedTransport::new([]));
    let lease = CredentialLease::new(
        producing.credential_ref.clone(), CredentialTarget::for_route(&producing), "fixture-handle",
    );
    adapter.encode_request(&request, &producing, &lease).unwrap();
    let wrong = OpenAiChatAdapter::new(ScriptedTransport::new([]));
    let lease = CredentialLease::new(
        primary.credential_ref.clone(), CredentialTarget::for_route(&primary), "fixture-handle",
    );
    let error = wrong.encode_request(&request, &primary, &lease).unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
    assert_eq!(error.retry, ChatRetryDirective::Never);
}

#[tokio::test]
async fn gemini_broker_rejects_decoder_supplied_tool_signature_origin() {
    let target = route(ChatProtocol::Gemini, "target-route", 1);
    for protocol in [ChatProtocol::Gemini, ChatProtocol::OpenaiChat] {
    let route = route(protocol, "remote-tool-origin", 1);
    let transport = ScriptedTransport::new([TransportScript::Frames(vec![
        frame("response.start", serde_json::json!({"id": "remote-origin"})),
        frame("tool_call.completed", serde_json::json!({
            "call_id": "remote-call", "name": "lookup", "arguments": {},
            "provider_metadata": {
                "thoughtSignature": "opaque-signature",
                "_nomifun_producing_route_digest": nomifun_agent_contracts::digest_payload(&target).unwrap()
            }
        })),
        frame("done", serde_json::json!({"finish_reason": "tool_calls"})),
    ])]);
    let transports = transport_map([(protocol, provider_transport(&transport))]);
    let broker = broker(
        StaticCausalityGate::allow(), ResolvedChatRouteSet { primary: route.clone(), failovers: vec![] },
        Arc::new(StaticCredentialStore { mismatch: false }), &transports,
        BrokerRetryPolicy { max_total_attempts: 1, max_attempts_per_route: 1 },
    );
    let events = broker.open_stream(basic_request(&route)).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(events.len(), 1);
    let error = events[0].as_ref().unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
    assert_eq!(error.retry, ChatRetryDirective::Never);
    }
}

#[tokio::test]
async fn gemini_signed_tool_history_cannot_fail_over_to_a_different_route() {
    let original = route(ChatProtocol::Gemini, "signed-primary", 1);
    let call = bound_gemini_tool_call(&original, "thoughtSignature").await;
    let request = gemini_tool_continuation(&original, call);
    let primary = ScriptedTransport::new([TransportScript::OpenError(ChatModelError::new(
        ChatModelErrorCode::ProviderUnavailable, "fixture failure", ChatRetryDirective::Failover,
    ))]);
    let failover = ScriptedTransport::new([TransportScript::Frames(successful_frames("must-not-open", "reply"))]);
    // Distinct protocols use independent scripted transports, proving that
    // a signed Gemini history cannot silently lose metadata during failover.
    let transports = transport_map([
        (ChatProtocol::Gemini, provider_transport(&primary)),
        (ChatProtocol::OpenaiChat, provider_transport(&failover)),
    ]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet { primary: original, failovers: vec![route(ChatProtocol::OpenaiChat, "signed-failover", 1)] },
        Arc::new(StaticCredentialStore { mismatch: false }), &transports,
        BrokerRetryPolicy { max_total_attempts: 2, max_attempts_per_route: 1 },
    );
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(primary.calls(), 1);
    assert_eq!(failover.calls(), 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].as_ref().unwrap_err().code, ChatModelErrorCode::ProviderUnavailable);
}

#[test]
fn gemini_unsigned_reasoning_preserves_thought_marker_and_signed_reasoning_fails_closed() {
    let adapter = GeminiAdapter::new(ScriptedTransport::new([]));
    let route = route(ChatProtocol::Gemini, "reasoning-history", 1);
    let lease = CredentialLease::new(
        route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle",
    );
    let mut request = basic_request(&route);
    request.input.messages.push(ChatMessage {
        role: ChatRole::Assistant, provider_round_id: None,
        content: vec![
            ChatContentPart::Reasoning {
                text: "unsigned thought".into(), signature: None, encrypted_content: None,
            },
            ChatContentPart::Text { text: "answer".into() },
        ],
    });
    let body = adapter.encode_request(&request, &route, &lease).unwrap().body;
    assert_eq!(body["contents"][1]["parts"], serde_json::json!([
        {"text": "unsigned thought", "thought": true},
        {"text": "answer"}
    ]));
    request.input.messages[1].content.insert(0, ChatContentPart::Reasoning {
        text: "signed thought".into(), signature: Some("opaque-reasoning-signature".into()),
        encrypted_content: None,
    });
    let error = adapter.encode_request(&request, &route, &lease).unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
    assert_eq!(error.retry, ChatRetryDirective::Never);
}

#[test]
fn generic_reasoning_signatures_are_rejected_by_every_protocol_adapter() {
    let transports = transport_map([]);
    for adapter in adapters(&transports) {
        let route = route(adapter.protocol(), "unknown-reasoning-origin", 1);
        let mut request = basic_request(&route);
        request.input.messages.push(ChatMessage {
            role: ChatRole::Assistant, provider_round_id: None,
            content: vec![ChatContentPart::Reasoning {
                text: "private thought".into(), signature: Some("unknown-origin-signature".into()),
                encrypted_content: None,
            }],
        });
        let lease = CredentialLease::new(
            route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle",
        );
        let error = adapter.encode_request(&request, &route, &lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature, "{:?}", adapter.protocol());
        assert_eq!(error.retry, ChatRetryDirective::Never);
    }
}

#[tokio::test]
async fn generic_reasoning_signature_rejection_precedes_causality_credentials_and_all_failover_transports() {
    struct CountingCredentials { calls: AtomicUsize }
    #[async_trait]
    impl ProviderCredentialStore for CountingCredentials {
        async fn lease(&self, credential_ref: &ProviderCredentialRef, target: &CredentialTarget)
            -> Result<CredentialLease, ChatModelError>
        {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(CredentialLease::new(credential_ref.clone(), target.clone(), "fixture-handle"))
        }
    }
    let primary = route(ChatProtocol::Gemini, "generic-signature-primary", 1);
    let mut request = basic_request(&primary);
    request.input.messages.push(ChatMessage {
        role: ChatRole::Assistant, provider_round_id: None,
        content: vec![ChatContentPart::Reasoning {
            text: "private thought".into(), signature: Some("unknown-origin-signature".into()),
            encrypted_content: None,
        }],
    });
    let scripted = ChatProtocol::ALL.into_iter().map(|protocol| (protocol,
        ScriptedTransport::new([TransportScript::Frames(successful_frames("must-not-open", "reply"))])
    )).collect::<Vec<_>>();
    let transports = transport_map(scripted.iter().map(|(protocol, transport)|
        (*protocol, provider_transport(transport))));
    let gate = StaticCausalityGate::allow();
    let credentials = Arc::new(CountingCredentials { calls: AtomicUsize::new(0) });
    let failovers = ChatProtocol::ALL.into_iter().filter(|protocol| *protocol != ChatProtocol::Gemini)
        .map(|protocol| route(protocol, &format!("generic-signature-{protocol:?}"), 1)).collect();
    let broker = broker(
        gate.clone(), ResolvedChatRouteSet { primary, failovers }, credentials.clone(), &transports,
        BrokerRetryPolicy { max_total_attempts: 6, max_attempts_per_route: 1 },
    );
    let error = match broker.open_stream(request).await {
        Err(error) => error,
        Ok(_) => panic!("unknown reasoning origin opened a broker stream"),
    };
    assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
    assert_eq!(error.retry, ChatRetryDirective::Never);
    assert_eq!(gate.calls.load(Ordering::Acquire), 0);
    assert_eq!(credentials.calls.load(Ordering::Acquire), 0);
    for (protocol, transport) in scripted {
        assert_eq!(transport.calls(), 0, "{protocol:?}");
    }
}

#[tokio::test]
async fn anthropic_typed_reasoning_round_trips_actual_broker_route_and_rejects_changed_route() {
    use nomifun_chat_model_broker::ChatProviderReasoning;

    for via_failover in [false, true] {
        let producing = route(ChatProtocol::Anthropic, "typed-reasoning-producer", 1);
        let failed_primary = route(ChatProtocol::OpenaiChat, "typed-reasoning-primary", 1);
        let frames = [
            ("message_start", serde_json::json!({"type":"message_start", "message": {
                "id":"typed-reasoning-response", "type":"message", "role":"assistant",
                "content":[], "stop_reason":null, "usage":{"input_tokens":3,"output_tokens":0}
            }})),
            ("content_block_start", serde_json::json!({"type":"content_block_start", "index":0,
                "content_block":{"type":"thinking","thinking":""}})),
            ("content_block_delta", serde_json::json!({"type":"content_block_delta", "index":0,
                "delta":{"type":"thinking_delta","thinking":"native thought"}})),
            ("content_block_delta", serde_json::json!({"type":"content_block_delta", "index":0,
                "delta":{"type":"signature_delta","signature":"opaque-signature"}})),
            ("content_block_stop", serde_json::json!({"type":"content_block_stop","index":0})),
            ("content_block_start", serde_json::json!({"type":"content_block_start", "index":1,
                "content_block":{"type":"redacted_thinking","data":"opaque-redacted-data"}})),
            ("content_block_stop", serde_json::json!({"type":"content_block_stop","index":1})),
            ("message_delta", serde_json::json!({"type":"message_delta",
                "delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}})),
            ("message_stop", serde_json::json!({"type":"message_stop"})),
        ];
        let transport = ScriptedTransport::new([TransportScript::Frames(
            frames.into_iter().map(|(event, data)| frame(event, data)).collect(),
        )]);
        let primary_transport = ScriptedTransport::new([TransportScript::OpenError(ChatModelError::new(
            ChatModelErrorCode::ProviderUnavailable, "fixture primary failure", ChatRetryDirective::Failover,
        ))]);
        let transports = transport_map([
            (ChatProtocol::Anthropic, provider_transport(&transport)),
            (ChatProtocol::OpenaiChat, provider_transport(&primary_transport)),
        ]);
        let routes = if via_failover {
            ResolvedChatRouteSet { primary: failed_primary, failovers: vec![producing.clone()] }
        } else {
            ResolvedChatRouteSet { primary: producing.clone(), failovers: vec![] }
        };
        let request = basic_request(&routes.primary);
        let producing_broker = broker(
            StaticCausalityGate::allow(), routes,
            Arc::new(StaticCredentialStore { mismatch: false }), &transports,
            BrokerRetryPolicy { max_total_attempts: 2, max_attempts_per_route: 1 },
        );
        let events = producing_broker.open_stream(request).await.unwrap()
            .map(|result| result.unwrap().event).collect::<Vec<_>>().await;
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed {
            finish_reason: ChatFinishReason::Completed
        })));
        let blocks = events.into_iter().filter_map(|event| match event {
            ChatModelEvent::ProviderReasoningBlock { block } => Some(block),
            _ => None,
        }).collect::<Vec<_>>();
        let origin = nomifun_agent_contracts::digest_payload(&producing).unwrap();
        assert_eq!(blocks, vec![
            ChatProviderReasoning::AnthropicThinking {
                text: "native thought".into(), signature: "opaque-signature".into(),
                route_digest: Some(origin.clone()),
            },
            ChatProviderReasoning::AnthropicRedactedThinking {
                data: "opaque-redacted-data".into(), route_digest: Some(origin),
            },
        ]);
        assert_eq!(transport.calls(), 1);
        assert_eq!(primary_transport.calls(), usize::from(via_failover));
        // Canonical serialization retains the Broker-bound origin for replay.
        let blocks: Vec<ChatProviderReasoning> =
            serde_json::from_value(serde_json::to_value(blocks).unwrap()).unwrap();
        let mut continuation = basic_request(&producing);
        continuation.input.messages.push(ChatMessage {
            role: ChatRole::Assistant, provider_round_id: None,
            content: blocks.into_iter().map(|block| ChatContentPart::ProviderReasoning { block }).collect(),
        });
        let adapter = AnthropicAdapter::new(ScriptedTransport::new([]));
        let lease = CredentialLease::new(
            producing.credential_ref.clone(), CredentialTarget::for_route(&producing), "fixture-handle",
        );
        let body = adapter.encode_request(&continuation, &producing, &lease).unwrap().body;
        assert_eq!(body["messages"][1]["content"], serde_json::json!([
            {"type":"thinking","thinking":"native thought","signature":"opaque-signature"},
            {"type":"redacted_thinking","data":"opaque-redacted-data"}
        ]));
        assert!(!body.to_string().contains("route_digest"));

        let mut changed = producing.clone();
        changed.model_route_revision += 1;
        let changed_lease = CredentialLease::new(
            changed.credential_ref.clone(), CredentialTarget::for_route(&changed), "fixture-handle",
        );
        let error = adapter.encode_request(&continuation, &changed, &changed_lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.retry, ChatRetryDirective::Never);
        let gate = StaticCausalityGate::allow();
        let mut switched = basic_request(&changed);
        switched.input = continuation.input;
        let changed_broker = broker(
            gate.clone(), ResolvedChatRouteSet { primary: changed, failovers: vec![] },
            Arc::new(StaticCredentialStore { mismatch: false }), &transports,
            BrokerRetryPolicy { max_total_attempts: 1, max_attempts_per_route: 1 },
        );
        let error = match changed_broker.open_stream(switched).await {
            Err(error) => error,
            Ok(_) => panic!("changed route accepted typed reasoning history"),
        };
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.retry, ChatRetryDirective::Never);
        assert_eq!(gate.calls.load(Ordering::Acquire), 0);
        assert_eq!(transport.calls(), 1, "changed route must not open another attempt");
    }
}

#[tokio::test]
async fn raw_responses_reasoning_round_trips_exactly_and_is_bound_to_the_producing_route() {
    use nomifun_chat_model_broker::ChatProviderReasoning;
    let producing = route(ChatProtocol::OpenaiResponses, "raw-responses-producer", 1);
    let raw = serde_json::json!({"id":"rs_native", "type":"reasoning", "status":"completed",
        "content":[{"type":"reasoning_text","text":"actual native reasoning"}],
        "provider_extension":{"mode":"preserve-exactly"}});
    let frames = [
        ("response.created", serde_json::json!({"type":"response.created","response":{"id":"resp_native","status":"in_progress"}})),
        ("response.output_item.added", serde_json::json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_native","type":"reasoning","content":[]}})),
        ("response.reasoning_text.delta", serde_json::json!({"type":"response.reasoning_text.delta","output_index":0,
            "item_id":"rs_native","content_index":0,"delta":"actual native reasoning"})),
        ("response.reasoning_text.done", serde_json::json!({"type":"response.reasoning_text.done","output_index":0,
            "item_id":"rs_native","content_index":0,"text":"actual native reasoning"})),
        ("response.output_item.done", serde_json::json!({"type":"response.output_item.done","output_index":0,"item":raw})),
        ("response.completed", serde_json::json!({"type":"response.completed","response":{
            "id":"resp_native","status":"completed","output":[raw]}})),
    ];
    let transport = ScriptedTransport::new([TransportScript::Frames(
        frames.into_iter().map(|(event, data)| frame(event, data)).collect(),
    )]);
    let transports = transport_map([(ChatProtocol::OpenaiResponses, provider_transport(&transport))]);
    let owner = broker(StaticCausalityGate::allow(),
        ResolvedChatRouteSet {primary:producing.clone(),failovers:vec![]},
        Arc::new(StaticCredentialStore {mismatch:false}), &transports,
        BrokerRetryPolicy {max_total_attempts:1,max_attempts_per_route:1});
    let events = owner.open_stream(basic_request(&producing)).await.unwrap()
        .map(|event| event.unwrap().event).collect::<Vec<_>>().await;
    let block = events.into_iter().find_map(|event| match event {
        ChatModelEvent::ProviderReasoningBlock {block} => Some(block), _ => None,
    }).expect("typed native reasoning block");
    assert_eq!(block, ChatProviderReasoning::OpenaiResponsesReasoning {
        item:raw.clone(),route_digest:Some(nomifun_agent_contracts::digest_payload(&producing).unwrap()),
    });
    let block: ChatProviderReasoning = serde_json::from_value(serde_json::to_value(block).unwrap()).unwrap();
    let mut continuation = basic_request(&producing);
    continuation.input.messages.push(ChatMessage {role:ChatRole::Assistant,provider_round_id:None,
        content:vec![ChatContentPart::ProviderReasoning {block}]});
    let adapter = OpenAiResponsesAdapter::new(ScriptedTransport::new([]));
    let lease = CredentialLease::new(producing.credential_ref.clone(), CredentialTarget::for_route(&producing), "fixture-handle");
    let body = adapter.encode_request(&continuation, &producing, &lease).unwrap().body;
    assert_eq!(body["input"][1], raw);
    assert!(!body.to_string().contains("route_digest"));
    assert!(body["input"][1].get("encrypted_content").is_none());
    assert!(body["input"][1].get("summary").is_none());
    for protocol in [ChatProtocol::OpenaiChat, ChatProtocol::Anthropic, ChatProtocol::OpenaiResponses] {
        let changed = route(protocol,"different-producing-route",2);
        let adapter: Box<dyn ChatProtocolAdapter> = match protocol {
            ChatProtocol::OpenaiChat => Box::new(OpenAiChatAdapter::new(ScriptedTransport::new([]))),
            ChatProtocol::Anthropic => Box::new(AnthropicAdapter::new(ScriptedTransport::new([]))),
            _ => Box::new(OpenAiResponsesAdapter::new(ScriptedTransport::new([]))),
        };
        let lease = CredentialLease::new(changed.credential_ref.clone(), CredentialTarget::for_route(&changed), "fixture-handle");
        let error = adapter.encode_request(&continuation, &changed, &lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.retry, ChatRetryDirective::Never);
    }
}

#[test]
fn gemini_empty_text_signature_frame_is_not_dropped() {
    let adapter = GeminiAdapter::new(ScriptedTransport::new([]));
    let events = adapter.decode_frame(ProviderWireFrame {
        diagnostic: None, event: "message".into(),
        data: serde_json::json!({
            "responseId": "signature-only-frame",
            "candidates": [{"content": {"role": "model", "parts": [
                {"text": "", "thought": true, "thoughtSignature": "opaque-signature"}
            ]}}]
        }),
    }).unwrap();
    assert!(events.iter().any(|event| matches!(event,
        ChatModelEvent::ReasoningSignature { signature } if signature == "opaque-signature")));
    assert!(!events.iter().any(|event| matches!(event,
        ChatModelEvent::ReasoningDelta { .. } | ChatModelEvent::OutputTextDelta { .. })));
}

#[test]
fn gemini_tool_signature_metadata_is_validated_without_merging_other_fields() {
    let fixture = recorded_conformance_fixtures().into_iter()
        .find(|fixture| fixture.protocol == ChatProtocol::Gemini).unwrap();
    let adapter = GeminiAdapter::new(ScriptedTransport::new([]));
    let lease = CredentialLease::new(
        fixture.route.credential_ref.clone(), CredentialTarget::for_route(&fixture.route), "fixture-handle",
    );
    let with_metadata = |metadata: serde_json::Value| {
        let mut request = fixture.request.clone();
        let part = request.input.messages.iter_mut().flat_map(|message| &mut message.content)
            .find(|part| matches!(part, ChatContentPart::ToolCall { .. })).unwrap();
        if let ChatContentPart::ToolCall { provider_metadata, .. } = part {
            *provider_metadata = Some(nomifun_agent_contracts::StrictJsonValue(metadata));
        }
        request
    };
    for metadata in [
        serde_json::json!(null), serde_json::json!([]), serde_json::json!("not an object"),
        serde_json::json!({"thoughtSignature": null}), serde_json::json!({"thoughtSignature": 42}),
        serde_json::json!({"thoughtSignature": ""}),
        serde_json::json!({"thoughtSignature": "one", "thought_signature": "two"}),
    ] {
        let error = adapter.encode_request(&with_metadata(metadata), &fixture.route, &lease).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
        assert_eq!(error.retry, ChatRetryDirective::Never);
    }
    for signature_field in ["thoughtSignature", "thought_signature"] {
        let mut metadata = serde_json::json!({"unrelated_field": {"text": "do not merge"}});
        metadata[signature_field] = serde_json::json!("opaque-tool-signature");
        metadata["_nomifun_producing_route_digest"] =
            serde_json::to_value(nomifun_agent_contracts::digest_payload(&fixture.route).unwrap()).unwrap();
        let body = adapter.encode_request(&with_metadata(metadata), &fixture.route, &lease).unwrap().body;
        let part = body["contents"].as_array().unwrap().iter()
            .flat_map(|content| content["parts"].as_array().unwrap())
            .find(|part| part.get("functionCall").is_some()).unwrap();
        assert_eq!(part["thoughtSignature"], "opaque-tool-signature");
        assert!(part.get("unrelated_field").is_none());
        assert!(part.get("_nomifun_producing_route_digest").is_none());
        assert!(part["functionCall"].get("thoughtSignature").is_none());
    }
}

#[test]
fn unsigned_tool_metadata_remains_compatible_with_each_protocol() {
    let transports = transport_map([]);
    let adapters = adapters(&transports);
    for fixture in recorded_conformance_fixtures() {
        let mut request = fixture.request;
        for part in request.input.messages.iter_mut().flat_map(|message| &mut message.content) {
            if let ChatContentPart::ToolCall { provider_metadata, .. } = part {
                *provider_metadata = Some(nomifun_agent_contracts::StrictJsonValue(
                    serde_json::json!({"unrelated_field": "unsigned-provider-metadata"}),
                ));
            }
        }
        let adapter = adapters.iter().find(|adapter| adapter.protocol() == fixture.protocol).unwrap();
        let lease = CredentialLease::new(
            fixture.route.credential_ref.clone(), CredentialTarget::for_route(&fixture.route), "fixture-handle",
        );
        adapter.encode_request(&request, &fixture.route, &lease).unwrap();
    }
}

#[test]
fn gemini_tool_result_without_matching_call_fails_closed() {
    let mut fixture = recorded_conformance_fixtures()
        .into_iter()
        .find(|fixture| fixture.protocol == ChatProtocol::Gemini)
        .expect("Gemini fixture");
    for message in &mut fixture.request.input.messages {
        for part in &mut message.content {
            if let ChatContentPart::ToolResult { call_id, .. } = part {
                *call_id =
                    nomifun_chat_model_broker::ToolCallId("unmatched-gemini-call".to_owned());
            }
        }
    }
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapter = GeminiAdapter::new(transports[&ChatProtocol::Gemini].clone());
    let lease = CredentialLease::new(
        fixture.route.credential_ref.clone(),
        CredentialTarget::for_route(&fixture.route),
        "opaque-fixture-handle",
    );
    let error = adapter
        .encode_request(&fixture.request, &fixture.route, &lease)
        .expect_err("unmatched Gemini function response must not encode");

    assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
    assert_eq!(
        error.message,
        "Gemini function response has no matching function call"
    );
}

#[test]
fn provider_output_default_omits_optional_wire_fields_and_required_protocols_fail_before_transport() {
    for adapter in adapters(&transport_map([])) {
        let protocol=adapter.protocol();let route=route(protocol,"output-default",1);
        let lease=CredentialLease::new(route.credential_ref.clone(),CredentialTarget::for_route(&route),"fixture-handle");
        let mut request=basic_request(&route);request.input.max_output_tokens=None;
        if matches!(protocol,ChatProtocol::Anthropic|ChatProtocol::Bedrock|ChatProtocol::Vertex) {
            let error=adapter.encode_request(&request,&route,&lease).unwrap_err();
            assert_eq!(error.code,ChatModelErrorCode::InvalidRequest);
            assert!(error.message.contains("explicit output token ceiling"));
        } else {
            let body=adapter.encode_request(&request,&route,&lease).unwrap().body;
            assert!(body.get("max_tokens").is_none()&&body.get("max_output_tokens").is_none());
            assert!(body["generationConfig"].get("maxOutputTokens").is_none());
        }
        request.input.max_output_tokens=Some(100_000);
        let body=adapter.encode_request(&request,&route,&lease).unwrap().body;
        let value=match protocol {ChatProtocol::OpenaiResponses=>&body["max_output_tokens"],ChatProtocol::Gemini=>&body["generationConfig"]["maxOutputTokens"],_=>&body["max_tokens"]};
        assert_eq!(value.as_u64(),Some(100_000));
    }
}

#[test]
fn gemini_explicit_reasoning_controls_are_encoded_or_rejected_without_loss() {
    use nomifun_chat_model_broker::{ChatReasoningRequest, ReasoningEffort, ReasoningSummary};
    let transport = ScriptedTransport::new([]);
    let adapter = GeminiAdapter::new(transport);
    let route = route(ChatProtocol::Gemini, "thinking-controls", 1);
    let lease = CredentialLease::new(route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle");
    let mut request = basic_request(&route);
    request.input.reasoning = Some(ChatReasoningRequest {
        effort: None, summary: ReasoningSummary::Auto, max_reasoning_tokens: Some(2048),
    });
    let body = adapter.encode_request(&request, &route, &lease).unwrap().body;
    assert_eq!(body["generationConfig"]["thinkingConfig"], serde_json::json!({"thinkingBudget":2048,"includeThoughts":true}));
    request.input.reasoning.as_mut().unwrap().effort = Some(ReasoningEffort::High);
    assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::InvalidRequest);
    request.input.reasoning.as_mut().unwrap().max_reasoning_tokens = None;
    request.input.reasoning.as_mut().unwrap().summary = ReasoningSummary::Detailed;
    assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
    request.input.reasoning = None;
    request.input.prompt_cache = PromptCachePolicy::Ephemeral;
    assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
}

#[test]
fn openai_separate_reasoning_budgets_and_chat_summary_details_fail_explicitly() {
    use nomifun_chat_model_broker::{ChatReasoningRequest, ReasoningEffort, ReasoningSummary};
    for adapter in adapters(&transport_map([])).into_iter()
        .filter(|adapter| matches!(adapter.protocol(), ChatProtocol::OpenaiChat | ChatProtocol::OpenaiResponses)) {
        let route = route(adapter.protocol(), "reasoning-budget", 1);
        let lease = CredentialLease::new(route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle");
        let mut request = basic_request(&route);
        request.input.reasoning = Some(ChatReasoningRequest {
            effort: Some(ReasoningEffort::High), summary: ReasoningSummary::None, max_reasoning_tokens: Some(2048),
        });
        assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
        if adapter.protocol() == ChatProtocol::OpenaiChat {
            request.input.reasoning.as_mut().unwrap().max_reasoning_tokens = None;
            request.input.reasoning.as_mut().unwrap().summary = ReasoningSummary::Detailed;
            assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
        }
    }
}

#[test]
fn official_reasoning_tiers_preserve_default_absence_and_explicit_disable() {
    use nomifun_chat_model_broker::{ChatReasoningRequest, ReasoningEffort, ReasoningSummary};
    for adapter in adapters(&transport_map([])).into_iter()
        .filter(|adapter| matches!(adapter.protocol(), ChatProtocol::OpenaiChat | ChatProtocol::OpenaiResponses | ChatProtocol::Gemini)) {
        let route = route(adapter.protocol(), "official-tiers", 1);
        let lease = CredentialLease::new(route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle");
        let mut request = basic_request(&route);
        let defaults = adapter.encode_request(&request, &route, &lease).unwrap().body;
        assert!(defaults.get("reasoning_effort").is_none());
        assert!(defaults.get("reasoning").is_none());
        assert!(defaults["generationConfig"].get("thinkingConfig").is_none());
        request.input.reasoning = Some(ChatReasoningRequest {
            effort: Some(ReasoningEffort::Minimal), summary: ReasoningSummary::None, max_reasoning_tokens: None,
        });
        let minimal = adapter.encode_request(&request, &route, &lease).unwrap().body;
        let effort = match adapter.protocol() {
            ChatProtocol::OpenaiChat => &minimal["reasoning_effort"],
            ChatProtocol::OpenaiResponses => &minimal["reasoning"]["effort"],
            ChatProtocol::Gemini => &minimal["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            _ => unreachable!(),
        };
        assert_eq!(effort, "minimal");
        request.input.reasoning.as_mut().unwrap().effort = Some(ReasoningEffort::None);
        assert!(!request.input.required_features().contains(&ChatModelFeature::Reasoning));
        if adapter.protocol() == ChatProtocol::Gemini {
            assert_eq!(adapter.encode_request(&request, &route, &lease).unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
        } else {
            let disabled = adapter.encode_request(&request, &route, &lease).unwrap().body;
            let effort = if adapter.protocol() == ChatProtocol::OpenaiChat { &disabled["reasoning_effort"] }
                else { &disabled["reasoning"]["effort"] };
            assert_eq!(effort, "none");
        }
    }
}

#[tokio::test]
async fn openai_chat_concise_summary_is_rejected_before_any_transport_call() {
    use nomifun_chat_model_broker::{ChatReasoningRequest, ReasoningEffort, ReasoningSummary};
    let primary = route(ChatProtocol::OpenaiChat, "unsupported-summary", 1);
    let mut request = basic_request(&primary);
    request.input.reasoning = Some(ChatReasoningRequest {
        effort: Some(ReasoningEffort::High), summary: ReasoningSummary::Concise, max_reasoning_tokens: None,
    });
    let transport = ScriptedTransport::new([]);
    let transports = transport_map([(ChatProtocol::OpenaiChat, provider_transport(&transport))]);
    let broker = broker(StaticCausalityGate::allow(), ResolvedChatRouteSet { primary, failovers: Vec::new() },
        Arc::new(StaticCredentialStore { mismatch: false }), &transports, BrokerRetryPolicy::default());
    let output = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(output.last().unwrap().as_ref().unwrap_err().code, ChatModelErrorCode::UnsupportedFeature);
    assert_eq!(transport.calls(), 0);
}

#[tokio::test]
async fn failover_output_bound_comes_from_actual_attempt_and_preserves_explicit_caller_bound() {
    for (primary_protocol, backup_protocol, primary_limit, backup_limit) in [
        (ChatProtocol::OpenaiChat, ChatProtocol::Anthropic, 4096, 100_000),
        (ChatProtocol::Anthropic, ChatProtocol::OpenaiChat, 100_000, 4096),
    ] {
        for caller_bound in [None, Some(512)] {
            let primary = route(primary_protocol, "bound-primary", 1);
            let backup = route(backup_protocol, "bound-backup", 1);
            let mut request = basic_request(&primary);
            request.input.max_output_tokens = caller_bound;
            let requests = Arc::new(Mutex::new(Vec::new()));
            let transport: Arc<dyn ProviderTransport> = Arc::new(OutputBoundCaptureTransport { requests: Arc::clone(&requests) });
            let transports = transport_map([(primary_protocol, Arc::clone(&transport)), (backup_protocol, transport)]);
            let resolver = OutputBoundRouteResolver {
                routes: ResolvedChatRouteSet { primary, failovers: vec![backup] },
                limits: BTreeMap::from([("bound-primary".into(), primary_limit), ("bound-backup".into(), backup_limit)]),
            };
            let broker = ChatModelBroker::new(StaticCausalityGate::allow(), Arc::new(resolver),
                Arc::new(StaticCredentialStore { mismatch: false }), adapters(&transports),
                BrokerRetryPolicy { max_total_attempts: 2, max_attempts_per_route: 1 }).unwrap();
            let output = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
            assert!(output.iter().all(Result::is_ok));
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            for (index, limit) in [primary_limit, backup_limit].into_iter().enumerate() {
                assert_eq!(requests[index].body["max_tokens"].as_u64(),
                    Some(u64::from(caller_bound.map_or(limit, |caller| caller.min(limit)))));
            }
        }
    }
}

#[tokio::test]
async fn required_attempt_without_caller_or_model_output_limit_fails_before_transport() {
    let primary = route(ChatProtocol::Anthropic, "missing-bound", 1);
    let mut request = basic_request(&primary);
    request.input.max_output_tokens = None;
    let transport = ScriptedTransport::new([]);
    let transports = transport_map([(ChatProtocol::Anthropic, provider_transport(&transport))]);
    let broker = broker(StaticCausalityGate::allow(), ResolvedChatRouteSet { primary, failovers: Vec::new() },
        Arc::new(StaticCredentialStore { mismatch: false }), &transports, BrokerRetryPolicy::default());
    let output = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    let error = output.last().unwrap().as_ref().unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
    assert!(error.message.contains("explicit output token ceiling"));
    assert_eq!(transport.calls(), 0);
}

#[test]
fn parallel_tool_delivery_preference_is_optional_and_uses_each_protocol_control() {
    for adapter in adapters(&transport_map([])) {
        let protocol = adapter.protocol();
        let route = route(protocol, "parallel-preference", 1);
        let lease = CredentialLease::new(route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle");
        let mut request = basic_request(&route);
        request.input.tools = vec![nomifun_chat_model_broker::ChatToolDefinition {
            name:"fixture_read".into(),description:"Read the fixture".into(),
            input_schema:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"type":"object","properties":{},"additionalProperties":false})),deferred:false,
        }];
        request.input.tool_choice = ChatToolChoice::Auto;
        for preference in [None,Some(false),Some(true)] {
            request.input.parallel_tool_calls = preference;
            let body = adapter.encode_request(&request,&route,&lease).unwrap().body;
            match protocol {
                ChatProtocol::OpenaiChat | ChatProtocol::OpenaiResponses => {
                    assert_eq!(body.get("parallel_tool_calls").and_then(serde_json::Value::as_bool),preference);
                    assert_eq!(body["tool_choice"],"auto");
                }
                ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
                    assert_eq!(body["tool_choice"].get("disable_parallel_tool_use").and_then(serde_json::Value::as_bool),preference.map(|value|!value));
                    assert_eq!(body["tool_choice"]["type"],"auto");
                    assert!(body.get("parallel_tool_calls").is_none());
                }
                ChatProtocol::Gemini => {
                    // This adapter has no corresponding wire control. The
                    // preference never substitutes for Runtime batch checks.
                    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"],"AUTO");
                    assert!(body.get("parallel_tool_calls").is_none());
                }
            }
        }
    }
}

#[test]
fn historical_model_inputs_default_to_no_parallel_preference_and_reject_non_booleans() {
    let input = basic_request(&route(ChatProtocol::OpenaiChat,"history",1)).input;
    let mut value = serde_json::to_value(&input).unwrap();
    assert!(value.get("parallel_tool_calls").is_none());
    assert_eq!(serde_json::from_value::<ChatModelInput>(value.clone()).unwrap(),input);
    value["parallel_tool_calls"]=serde_json::json!(false);
    assert_eq!(serde_json::from_value::<ChatModelInput>(value.clone()).unwrap().parallel_tool_calls,Some(false));
    value["parallel_tool_calls"]=serde_json::json!("false");
    assert!(serde_json::from_value::<ChatModelInput>(value).is_err());
}

#[test]
fn completion_schema_changes_reach_openai_wire_without_caching_or_sanitizing() {
    let route = route(ChatProtocol::OpenaiChat, "completion-schema", 1);
    let lease = CredentialLease::new(route.credential_ref.clone(), CredentialTarget::for_route(&route), "fixture-handle");
    let adapter = OpenAiChatAdapter::new(ScriptedTransport::new([]));
    let mut request = basic_request(&route);
    let schema = serde_json::json!({"type":"object","additionalProperties":false,"properties":{
        "summary":{"type":"string","description":"The only final answer delivered to the user"},
        "criteria":{"type":"array","items":{"type":"object","properties":{
            "evidence_paths":{"type":"array","maxItems":8,"items":{"type":"string","enum":["验收/回执.txt"]}}
        }}}
    }});
    request.input.tools = vec![nomifun_chat_model_broker::ChatToolDefinition {
        name:"report_completion".into(),description:"Finish and deliver the exact final answer".into(),
        input_schema:nomifun_agent_contracts::StrictJsonValue(schema.clone()),deferred:false,
    }];
    request.input.tool_choice = ChatToolChoice::Auto;
    let before = adapter.encode_request(&request, &route, &lease).unwrap();
    assert_eq!(before.body["tools"][0]["function"]["parameters"], schema);
    let path_schema=&mut request.input.tools[0].input_schema.0["properties"]["criteria"]["items"]["properties"]["evidence_paths"];
    path_schema["maxItems"]=serde_json::json!(0);
    path_schema["items"].as_object_mut().unwrap().remove("enum");
    let after = adapter.encode_request(&request, &route, &lease).unwrap();
    assert_eq!(after.body["tools"][0]["function"]["parameters"],request.input.tools[0].input_schema.0);
    assert_ne!(before.body["tools"],after.body["tools"]);
    let bytes=serde_json::to_vec(&after.body).unwrap();
    let decoded:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded["tools"],after.body["tools"]);
}

#[test]
fn adapters_are_single_attempt_and_request_bodies_contain_no_credentials() {
    let transports =
        transport_map(std::iter::empty::<(ChatProtocol, Arc<dyn ProviderTransport>)>());
    let adapters = adapters(&transports)
        .into_iter()
        .map(|adapter| (adapter.protocol(), adapter))
        .collect::<BTreeMap<_, _>>();

    for fixture in recorded_conformance_fixtures() {
        let adapter = &adapters[&fixture.protocol];
        assert_eq!(adapter.retry_count(), 0);
        let target = CredentialTarget::for_route(&fixture.route);
        let lease = CredentialLease::new(
            fixture.route.credential_ref.clone(),
            target,
            "opaque-fixture-handle",
        );
        let request = adapter
            .encode_request(&fixture.request, &fixture.route, &lease)
            .expect("fixture must encode");
        assert_eq!(request.protocol, fixture.route.protocol);
        assert_eq!(request.route_identity.route_id, fixture.route.model_route_id);
        assert_eq!(
            request.route_identity.route_revision,
            fixture.route.model_route_revision
        );
        assert_eq!(
            request.connection_config_ref,
            fixture.route.connection_config_ref
        );
        assert_eq!(
            request.config_revision_digest,
            fixture.route.config_revision_digest
        );
        assert_eq!(request.credential_ref, fixture.route.credential_ref);
        assert_recorded_request_shape(fixture.protocol, &request.body);
        let serialized = request.body.to_string().to_ascii_lowercase();
        for forbidden in [
            "credential-ref-",
            "opaque-fixture-handle",
            "\"api_key\"",
            "\"authorization\"",
            "\"access_token\"",
            "\"client_secret\"",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "{:?} body exposed {forbidden}",
                fixture.protocol
            );
        }
    }
}

fn assert_recorded_request_shape(protocol: ChatProtocol, body: &serde_json::Value) {
    assert!(
        body.get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "{protocol:?} fixture omitted tool definitions"
    );
    let serialized = body.to_string();
    match protocol {
        ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
            assert!(serialized.contains("\"type\":\"tool_result\""));
            assert!(serialized.contains("\"tool_use_id\":\"call-history-"));
            assert!(body.get("tool_choice").is_some());
        }
        ChatProtocol::OpenaiChat => {
            assert!(serialized.contains("\"role\":\"tool\""));
            assert!(serialized.contains("\"tool_call_id\":\"call-history-"));
            assert!(body.get("tool_choice").is_some());
        }
        ChatProtocol::OpenaiResponses => {
            assert!(serialized.contains("\"type\":\"function_call_output\""));
            assert!(body.get("tool_choice").is_some());
            assert!(body.get("previous_response_id").is_some());
        }
        ChatProtocol::Gemini => {
            assert!(serialized.contains("\"functionResponse\""));
            assert!(body.get("toolConfig").is_some());
        }
    }
}

#[tokio::test]
async fn broker_reports_only_conclusive_pre_semantic_capability_failures() {
    let route = route(ChatProtocol::OpenaiChat, "observed", 1);
    let request = basic_request(&route);
    let mut unsupported = ChatModelError::new(
        ChatModelErrorCode::UnsupportedFeature,
        "provider machine error",
        ChatRetryDirective::Never,
    );
    unsupported.unsupported_feature = Some(ChatModelFeature::ToolCalls);
    let transport = ScriptedTransport::new([TransportScript::OpenError(unsupported)]);
    let transports = transport_map([(
        ChatProtocol::OpenaiChat,
        provider_transport(&transport),
    )]);
    let observer = Arc::new(RecordingCapabilityObserver::default());
    let broker = broker_with_observer(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: route,
            failovers: Vec::new(),
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy {
            max_total_attempts: 1,
            max_attempts_per_route: 1,
        },
        observer.clone(),
    );
    let events = broker
        .open_chat_stream(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().any(Result::is_err));
    assert_eq!(
        *observer.observations.lock().unwrap(),
        vec![("observed".to_owned(), ChatModelFeature::ToolCalls)]
    );
}

#[tokio::test]
async fn rejected_reasoning_tier_keeps_feature_health_and_surfaces_parameter_error() {
    let primary = route(ChatProtocol::OpenaiChat, "unsupported-tier", 1);
    let request = basic_request(&primary);
    let transport = ScriptedTransport::new([TransportScript::Frames(vec![frame("error", serde_json::json!({
        "error":{"code":"unsupported_value","param":"reasoning_effort","message":"unsupported tier"}
    }))])]);
    let transports = transport_map([(ChatProtocol::OpenaiChat, provider_transport(&transport))]);
    let observer = Arc::new(RecordingCapabilityObserver::default());
    let broker = broker_with_observer(StaticCausalityGate::allow(),
        ResolvedChatRouteSet { primary, failovers: Vec::new() },
        Arc::new(StaticCredentialStore { mismatch: false }), &transports,
        BrokerRetryPolicy { max_total_attempts: 1, max_attempts_per_route: 1 }, observer.clone());
    let output = broker.open_chat_stream(request).await.unwrap().collect::<Vec<_>>().await;
    let error = output.last().unwrap().as_ref().unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
    assert_eq!(error.retry, ChatRetryDirective::Never);
    assert!(error.unsupported_feature.is_none());
    assert!(observer.observations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn broker_discards_failed_pre_semantic_attempt_and_fails_over_once() {
    let primary_route = route(ChatProtocol::Anthropic, "primary", 1);
    let failover_route = route(ChatProtocol::OpenaiChat, "failover", 1);
    let request = basic_request(&primary_route);
    let primary = ScriptedTransport::new([TransportScript::Frames(vec![
        frame("message_start", serde_json::json!({"message": {"id": "discarded"}})),
        Err(ChatModelError::provider_unavailable(
            "empty stream transport reset",
        )),
    ])]);
    let failover = ScriptedTransport::new([TransportScript::Frames(successful_frames(
        "committed",
        "hello from failover",
    ))]);
    let transports = transport_map([
        (
            ChatProtocol::Anthropic,
            provider_transport(&primary),
        ),
        (
            ChatProtocol::OpenaiChat,
            provider_transport(&failover),
        ),
    ]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: primary_route,
            failovers: vec![failover_route],
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy {
            max_total_attempts: 2,
            max_attempts_per_route: 1,
        },
    );

    let output = broker
        .open_stream(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let events = output
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("failover stream");
    assert_eq!(primary.calls(), 1);
    assert_eq!(failover.calls(), 1);
    assert!(
        events
            .iter()
            .all(|event| event.protocol == ChatProtocol::OpenaiChat)
    );
    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            ChatModelEvent::OutputTextDelta { text, .. } if text == "hello from failover"
        )
    }));
    assert!(
        events
            .last()
            .is_some_and(|event| event.event.is_terminal())
    );
}

#[tokio::test]
async fn final_failure_diagnostic_uses_actual_failover_provider_and_model() {
    use nomifun_agent_contracts::{ModelFailureDiagnostic, ModelFailureReason};
    let mut native = frame("error", serde_json::json!({"error": {
        "code": "invalid_api_key", "message": "private native diagnostic"
    }})).unwrap();
    let mut context = ModelFailureDiagnostic::new(ModelFailureReason::AuthFailed);
    context.provider_id = Some("stale-selected-provider".to_owned());
    context.model_name = Some("stale-selected-model".to_owned());
    native.diagnostic = Some(context);
    for (frames, reason, code, semantic) in [
        (vec![Ok(native)], ModelFailureReason::InvalidKey, ChatModelErrorCode::AuthenticationFailed, false),
        (vec![frame("message", serde_json::json!({"choices": "malformed"}))],
            ModelFailureReason::InvalidResponse, ChatModelErrorCode::ProtocolViolation, false),
        (vec![], ModelFailureReason::StreamInterrupted, ChatModelErrorCode::StreamInterrupted, false),
        (vec![frame("message", serde_json::json!({"choices": [{"index": 0,
                "delta": {"content": "committed output"}, "finish_reason": null}]})),
            frame("message", serde_json::json!({"choices": "malformed"}))],
            ModelFailureReason::InvalidResponse, ChatModelErrorCode::ProtocolViolation, true),
    ] {
        let primary_route = route(ChatProtocol::Anthropic, "selected-primary", 1);
        let failover_route = route(ChatProtocol::OpenaiChat, "actual-failure", 1);
        let request = basic_request(&primary_route);
        let primary = ScriptedTransport::new([TransportScript::Frames(vec![frame("error",
            serde_json::json!({"error": {"type": "overloaded_error"}}))])]);
        let failover = ScriptedTransport::new([TransportScript::Frames(frames)]);
        let transports = transport_map([
            (ChatProtocol::Anthropic, provider_transport(&primary)),
            (ChatProtocol::OpenaiChat, provider_transport(&failover)),
        ]);
        let broker = broker(StaticCausalityGate::allow(), ResolvedChatRouteSet {
            primary: primary_route, failovers: vec![failover_route.clone()],
        }, Arc::new(StaticCredentialStore { mismatch: false }), &transports,
            BrokerRetryPolicy { max_total_attempts: 2, max_attempts_per_route: 1 });
        let output = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
        assert_eq!(primary.calls(), 1);
        assert_eq!(failover.calls(), 1);
        let error = output.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.code, code);
        let diagnostic = error.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.reason, reason);
        assert_eq!(diagnostic.provider_id.as_deref(), Some(failover_route.provider_id.as_ref()));
        assert_eq!(diagnostic.model_name.as_deref(), Some(failover_route.model.as_str()));
        assert_eq!(error.semantic_output_committed, semantic);
        if semantic {
            assert_eq!(error.retry, ChatRetryDirective::Never);
        }
        assert_eq!(diagnostic.http_status, None);
        assert_eq!(diagnostic.endpoint, None);
    }
}

#[tokio::test]
async fn broker_never_switches_route_after_semantic_output() {
    let primary_route = route(ChatProtocol::Anthropic, "primary-semantic", 1);
    let failover_route = route(ChatProtocol::OpenaiChat, "unused-failover", 1);
    let request = basic_request(&primary_route);
    let primary = ScriptedTransport::new([TransportScript::Frames(vec![
        frame("message_start", serde_json::json!({"message": {"id": "committed"}})),
        frame("text.delta", serde_json::json!({"text": "committed text"})),
        Err(ChatModelError::provider_unavailable(
            "stream reset after semantic output",
        )),
    ])]);
    let failover = ScriptedTransport::new([TransportScript::Frames(successful_frames(
        "must-not-run",
        "duplicate output",
    ))]);
    let transports = transport_map([
        (
            ChatProtocol::Anthropic,
            provider_transport(&primary),
        ),
        (
            ChatProtocol::OpenaiChat,
            provider_transport(&failover),
        ),
    ]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: primary_route,
            failovers: vec![failover_route],
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );

    let output = broker
        .open_stream(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(primary.calls(), 1);
    assert_eq!(failover.calls(), 0);
    assert!(output.iter().any(|item| matches!(
        item,
        Ok(event) if matches!(
            &event.event,
            ChatModelEvent::OutputTextDelta { text, .. } if text == "committed text"
        )
    )));
    let error = output.last().unwrap().as_ref().unwrap_err();
    assert!(error.semantic_output_committed);
    assert_eq!(error.retry, ChatRetryDirective::Never);
}

#[tokio::test]
async fn broker_owns_bounded_same_route_retry() {
    let primary_route = route(ChatProtocol::Anthropic, "same-route", 1);
    let request = basic_request(&primary_route);
    let primary = ScriptedTransport::new([
        TransportScript::OpenError(ChatModelError::new(
            ChatModelErrorCode::RateLimited,
            "retry this route once",
            ChatRetryDirective::RetrySameRoute,
        )),
        TransportScript::Frames(successful_frames("same-route-ok", "retried once")),
    ]);
    let transports = transport_map([(
        ChatProtocol::Anthropic,
        provider_transport(&primary),
    )]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: primary_route,
            failovers: Vec::new(),
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy {
            max_total_attempts: 2,
            max_attempts_per_route: 2,
        },
    );

    let output = broker
        .open_stream(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(output.iter().all(Result::is_ok));
    assert_eq!(primary.calls(), 2);
    assert!(
        output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .all(|event| event.total_attempt == 2 && event.route_attempt == 2)
    );
}

fn single_route_retry_broker(
    scripts: impl IntoIterator<Item = TransportScript>,
) -> (Arc<ChatModelBroker>, ChatModelRequest, Arc<ScriptedTransport>) {
    let primary_route = route(ChatProtocol::Anthropic, "retry-fixture", 1);
    let request = basic_request(&primary_route);
    let transport = ScriptedTransport::new(scripts);
    let transports = transport_map([(
        ChatProtocol::Anthropic,
        provider_transport(&transport),
    )]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet { primary: primary_route, failovers: Vec::new() },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    (broker, request, transport)
}

#[tokio::test(start_paused = true)]
async fn gateway_business_failures_never_switch_configured_route() {
    use nomifun_net::provider_gateway_error::GatewayBusinessError;

    for business in [GatewayBusinessError::InsufficientBalance,
        GatewayBusinessError::SubscriptionExpired, GatewayBusinessError::ModelNotInPlan,
        GatewayBusinessError::KeyExpired, GatewayBusinessError::RateLimited] {
        let primary_route = route(ChatProtocol::OpenaiChat, "gateway-primary", 1);
        let request = basic_request(&primary_route);
        let primary = ScriptedTransport::new((0..3).map(|_| {
            TransportScript::Frames(vec![frame("error", serde_json::json!({
                "error": {"code": business.code(), "message": "private upstream detail"}
            }))])
        }));
        let failover = ScriptedTransport::new([TransportScript::Frames(successful_frames(
            "must-not-open", "another model must not bypass a gateway business failure",
        ))]);
        let transports = transport_map([
            (ChatProtocol::OpenaiChat, provider_transport(&primary)),
            (ChatProtocol::Anthropic, provider_transport(&failover)),
        ]);
        let broker = broker(
            StaticCausalityGate::allow(),
            ResolvedChatRouteSet {
                primary: primary_route,
                failovers: vec![route(ChatProtocol::Anthropic, "gateway-failover", 1)],
            },
            Arc::new(StaticCredentialStore { mismatch: false }),
            &transports,
            BrokerRetryPolicy { max_total_attempts: 4, max_attempts_per_route: 2 },
        );
        let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
        assert_eq!(primary.calls(), if business == GatewayBusinessError::RateLimited { 2 } else { 1 });
        assert_eq!(failover.calls(), 0, "{}", business.code());
        assert_eq!(events.len(), 1);
        let error = events[0].as_ref().unwrap_err();
        assert_eq!(error.message, business.action_message());
        assert_eq!(error.diagnostic.as_ref().unwrap().provider_code.as_deref(), Some(business.code()));
    }
}

async fn wait_for_attempt(transport: &ScriptedTransport, count: usize) {
    // Runnable yields do not advance Tokio's paused clock. Waiting for a
    // transport attempt must not accidentally elapse the retry deadline.
    for _ in 0..100 {
        if transport.calls() >= count { return; }
        tokio::task::yield_now().await;
    }
    panic!("provider attempt did not start");
}

#[tokio::test(start_paused = true)]
async fn single_route_recovers_two_transient_provider_errors_without_exposing_failed_attempts() {
    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::Frames(vec![frame("error", serde_json::json!({
            "error":{"type":"rate_limit_error"}
        }))]),
        TransportScript::Frames(vec![frame("error", serde_json::json!({
            "error":{"type":"overloaded_error"}
        }))]),
        TransportScript::Frames(successful_frames("third-attempt", "recovered")),
    ]);
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), 3);
    assert!(events.iter().all(Result::is_ok));
    assert!(events.last().unwrap().as_ref().unwrap().event.is_terminal());
    assert!(events.iter().all(|item| item.as_ref().is_ok_and(|event|
        event.total_attempt == 3 && event.route_attempt == 3)));
}

#[tokio::test(start_paused = true)]
async fn single_route_retries_an_in_stream_gateway_upstream_error_before_output() {
    let primary_route = route(ChatProtocol::OpenaiChat, "upstream-error-fixture", 1);
    let request = basic_request(&primary_route);
    let transport = ScriptedTransport::new([
        TransportScript::Frames(vec![frame("message", serde_json::json!({
            "error":{"message":"upstream failed","type":"upstream_error","param":"","code":null}
        }))]),
        TransportScript::Frames(successful_frames("second-attempt", "recovered")),
    ]);
    let transports = transport_map([(
        ChatProtocol::OpenaiChat,
        provider_transport(&transport),
    )]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet { primary: primary_route, failovers: Vec::new() },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), 2);
    assert!(events.iter().all(Result::is_ok));
    assert!(events.last().unwrap().as_ref().unwrap().event.is_terminal());
}

#[tokio::test(start_paused = true)]
async fn single_route_retry_waits_for_retry_after_before_opening_provider_again() {
    let mut error = ChatModelError::new(
        ChatModelErrorCode::RateLimited, "cooldown", ChatRetryDirective::RetrySameRoute,
    );
    error.retry_after_ms = Some(7_000);
    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::OpenError(error),
        TransportScript::Frames(successful_frames("after-cooldown", "recovered")),
    ]);
    let stream = broker.open_stream(request).await.unwrap();
    wait_for_attempt(&transport, 1).await;
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(transport.calls(), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    let events = stream.collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), 2);
    assert!(events.iter().all(Result::is_ok));
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_retry_after_never_opens_a_second_attempt() {
    let mut error = ChatModelError::new(
        ChatModelErrorCode::RateLimited, "cooldown", ChatRetryDirective::RetrySameRoute,
    );
    error.retry_after_ms = Some(60_000);
    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::OpenError(error),
        TransportScript::Frames(successful_frames("must-not-open", "duplicate")),
    ]);
    let cancellation = CancellationToken::new();
    let stream = broker.open_chat_stream_cancellable(request, cancellation.clone()).await.unwrap();
    wait_for_attempt(&transport, 1).await;
    cancellation.cancel();
    let _ = stream.collect::<Vec<_>>().await;
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(transport.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn excessive_retry_after_is_returned_without_shortening_the_server_cooldown() {
    let mut error = ChatModelError::new(
        ChatModelErrorCode::RateLimited, "cooldown", ChatRetryDirective::RetrySameRoute,
    );
    error.retry_after_ms = Some(120_001);
    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::OpenError(error),
        TransportScript::Frames(successful_frames("must-not-open", "duplicate")),
    ]);
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].as_ref().unwrap_err().retry_after_ms, Some(120_001));
}

#[tokio::test(start_paused = true)]
async fn repeated_transient_failure_exhausts_bounded_attempts_not_the_task_forever() {
    let (broker, request, transport) = single_route_retry_broker((0..10).map(|_| {
        TransportScript::OpenError(ChatModelError::new(
            ChatModelErrorCode::ProviderUnavailable, "503", ChatRetryDirective::RetrySameRoute,
        ))
    }));
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), usize::from(BrokerRetryPolicy::default().max_attempts_per_route));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].as_ref().unwrap_err().code, ChatModelErrorCode::ProviderUnavailable);
}

#[tokio::test(start_paused = true)]
async fn authentication_quota_and_invalid_parameters_never_retry_same_route() {
    for code in ["authentication_error", "insufficient_quota", "invalid_request_error"] {
        let (broker, request, transport) = single_route_retry_broker([
            TransportScript::Frames(vec![frame("error", serde_json::json!({
                "error":{"type":code}
            }))]),
            TransportScript::Frames(successful_frames("must-not-open", "duplicate")),
        ]);
        let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
        assert_eq!(transport.calls(), 1, "{code}");
        assert_eq!(events.len(), 1, "{code}");
        assert_eq!(events[0].as_ref().unwrap_err().retry, ChatRetryDirective::Never);
    }
}

#[tokio::test(start_paused = true)]
async fn empty_disconnected_stream_retries_but_committed_text_does_not() {
    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::Frames(vec![]),
        TransportScript::Frames(successful_frames("after-disconnect", "recovered")),
    ]);
    assert!(broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await.iter().all(Result::is_ok));
    assert_eq!(transport.calls(), 2);

    let (broker, request, transport) = single_route_retry_broker([
        TransportScript::Frames(vec![
            frame("text.delta", serde_json::json!({"text":"already delivered"})),
            Err(ChatModelError::new(
                ChatModelErrorCode::ProviderUnavailable, "503", ChatRetryDirective::RetrySameRoute,
            )),
        ]),
        TransportScript::Frames(successful_frames("must-not-open", "duplicate")),
    ]);
    let events = broker.open_stream(request).await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(transport.calls(), 1);
    let error = events.last().unwrap().as_ref().unwrap_err();
    assert!(error.semantic_output_committed);
    assert_eq!(error.retry, ChatRetryDirective::Never);
}

#[tokio::test]
async fn unsupported_features_do_not_claim_the_operation() {
    for route_advertises_audio in [false, true] {
        let mut primary_route = route(ChatProtocol::Anthropic, "unsupported-audio", 1);
        if route_advertises_audio {
            primary_route.features.insert(ChatModelFeature::AudioOutput);
        }
        let mut request = basic_request(&primary_route);
        request.input.requested_output_modalities.insert(ChatModality::Audio);
        let gate = StaticCausalityGate::allow();
        let transport = ScriptedTransport::new([]);
        let transports = transport_map([(
            ChatProtocol::Anthropic,
            provider_transport(&transport),
        )]);
        let broker = broker(
            gate.clone(),
            ResolvedChatRouteSet { primary: primary_route, failovers: Vec::new() },
            Arc::new(StaticCredentialStore { mismatch: false }),
            &transports,
            BrokerRetryPolicy::default(),
        );
        let error = match broker.open_stream(request).await {
            Ok(_) => panic!("unsupported audio must be rejected before stream creation"),
            Err(error) => error,
        };
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(gate.calls.load(Ordering::Acquire), 0);
        assert_eq!(transport.calls(), 0);
    }
}

#[tokio::test]
async fn conditional_vision_route_is_used_only_for_image_requests() {
    let mut primary = route(ChatProtocol::Anthropic, "primary-text", 1);
    primary.features.remove(&ChatModelFeature::ImageInput);
    let mut vision = route(ChatProtocol::Anthropic, "vision-only", 1);
    vision.features.insert(ChatModelFeature::ImageInput);
    vision
        .activation_features
        .insert(ChatModelFeature::ImageInput);

    for wants_image in [false, true] {
        let mut request = basic_request(&primary);
        if wants_image {
            request.input.messages[0].content.push(ChatContentPart::Image {
                media_type: "image/png".into(),
                data_base64: "aA==".into(),
            });
        }
        let transport = ScriptedTransport::new([TransportScript::Frames(successful_frames(
            "conditional-route",
            "ok",
        ))]);
        let transports = transport_map([(
            ChatProtocol::Anthropic,
            provider_transport(&transport),
        )]);
        let broker = broker(
            StaticCausalityGate::allow(),
            ResolvedChatRouteSet {
                primary: primary.clone(),
                failovers: vec![vision.clone()],
            },
            Arc::new(StaticCredentialStore { mismatch: false }),
            &transports,
            BrokerRetryPolicy::default(),
        );
        let output = broker
            .open_stream(request)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        let route_id = output
            .iter()
            .find_map(|item| item.as_ref().ok().map(|event| event.route_id.as_ref()))
            .unwrap();
        assert_eq!(
            route_id,
            if wants_image { "vision-only" } else { "primary-text" }
        );
    }
}

#[tokio::test]
async fn dropping_broker_or_bridge_stream_releases_pending_provider_stream() {
    for through_bridge in [false, true] {
        let primary_route = route(ChatProtocol::Anthropic, "cancelled-stream", 1);
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(Notify::new());
        let transport = ScriptedTransport::new([TransportScript::PendingStream {
            started: started.clone(),
            dropped: dropped.clone(),
        }]);
        let transports = transport_map([(
            ChatProtocol::Anthropic,
            provider_transport(&transport),
        )]);
        let broker = broker(
            StaticCausalityGate::allow(),
            ResolvedChatRouteSet {
                primary: primary_route.clone(),
                failovers: vec![route(ChatProtocol::Anthropic, "unused-failover", 1)],
            },
            Arc::new(StaticCredentialStore { mismatch: false }),
            &transports,
            BrokerRetryPolicy::default(),
        );
        let output: Pin<Box<dyn Stream<Item = ()> + Send>> = if through_bridge {
            Box::pin(ResponsesBridge::new(broker)
                .open_stream(responses_request(&primary_route))
                .await.unwrap().map(|_| ()))
        } else {
            Box::pin(broker.open_stream(basic_request(&primary_route))
                .await.unwrap().map(|_| ()))
        };
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await.expect("provider stream must be polled before cancellation");
        drop(output);
        tokio::time::timeout(Duration::from_secs(2), dropped.notified())
            .await.expect("dropping output must release the pending provider stream");
        assert_eq!(transport.calls(), 1, "cancellation must not retry or fail over");
    }
}

#[tokio::test]
async fn native_cancellation_drops_provider_open_and_stream_without_consumer_polling() {
    for pending_open in [true, false] {
        for drop_consumer in [true, false] {
            let primary_route = route(ChatProtocol::Anthropic, "cancel-attempt", 1);
            let started = Arc::new(Notify::new());
            let dropped = Arc::new(Notify::new());
            let script = if pending_open {
                TransportScript::PendingOpen { started: started.clone(), dropped: dropped.clone() }
            } else {
                TransportScript::PendingStream { started: started.clone(), dropped: dropped.clone() }
            };
            let transport = ScriptedTransport::new([script]);
            let transports = transport_map([(ChatProtocol::Anthropic, provider_transport(&transport))]);
            let broker = broker(
                StaticCausalityGate::allow(),
                ResolvedChatRouteSet {
                    primary: primary_route.clone(),
                    failovers: vec![route(ChatProtocol::Anthropic, "unused-failover", 1)],
                },
                Arc::new(StaticCredentialStore { mismatch: false }),
                &transports,
                BrokerRetryPolicy::default(),
            );
            let parent = CancellationToken::new();
            let cancellation = parent.child_token();
            let mut output = Some(broker.open_chat_stream_cancellable(
                basic_request(&primary_route), cancellation.clone(),
            ).await.unwrap());
            tokio::time::timeout(Duration::from_secs(2), started.notified()).await.unwrap();
            if drop_consumer {
                drop(output.take());
                assert!(!cancellation.is_cancelled(), "stream drop must not cancel its caller");
            } else {
                cancellation.cancel();
            }
            // Keep the receiver alive and unpolled: cancellation must be owned
            // by the Broker task, not by the next downstream poll.
            tokio::time::timeout(Duration::from_secs(2), dropped.notified()).await
                .expect("cancellation must drop the actual provider attempt");
            if let Some(mut output) = output {
                assert!(output.next().await.is_none());
            }
            assert!(!parent.is_cancelled());
            assert_eq!(transport.calls(), 1, "cancel must never retry or fail over");
        }
    }
}

#[tokio::test]
async fn pre_cancelled_request_does_not_claim_causality_or_open_provider() {
    let primary = route(ChatProtocol::Anthropic, "pre-cancelled", 1);
    let gate = StaticCausalityGate::allow();
    let transport = ScriptedTransport::new([]);
    let transports = transport_map([(ChatProtocol::Anthropic, provider_transport(&transport))]);
    let broker = broker(
        gate.clone(),
        ResolvedChatRouteSet { primary: primary.clone(), failovers: vec![] },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = broker.open_chat_stream_cancellable(basic_request(&primary), cancellation)
        .await.err().expect("pre-cancelled request must fail");
    assert_eq!(error.code, ChatModelErrorCode::Cancelled);
    assert_eq!(error.retry, ChatRetryDirective::Never);
    assert_eq!(gate.calls.load(Ordering::Acquire), 0);
    assert_eq!(transport.calls(), 0);
}

struct PendingCredentialStore {
    started: Arc<Notify>,
    dropped: Arc<Notify>,
}

struct PendingRouteResolver {
    started: Arc<Notify>,
    dropped: Arc<Notify>,
}

#[async_trait]
impl ChatRouteResolver for PendingRouteResolver {
    async fn resolve(&self, _: &ChatRouteSelection) -> Result<ResolvedChatRouteSet, ChatModelError> {
        let _guard = NotifyOnDrop(self.dropped.clone());
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn native_cancellation_interrupts_route_preparation_before_causality_claim() {
    let primary = route(ChatProtocol::Anthropic, "pending-route", 1);
    let gate = StaticCausalityGate::allow();
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(Notify::new());
    let broker = Arc::new(ChatModelBroker::new(
        gate.clone(),
        Arc::new(PendingRouteResolver { started: started.clone(), dropped: dropped.clone() }),
        Arc::new(StaticCredentialStore { mismatch: false }),
        adapters(&transport_map([])),
        BrokerRetryPolicy::default(),
    ).unwrap());
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        broker.open_chat_stream_cancellable(basic_request(&primary), task_cancellation).await
    });
    tokio::time::timeout(Duration::from_secs(2), started.notified()).await.unwrap();
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap()
        .unwrap().err().expect("cancelled preparation must fail");
    assert_eq!(error.code, ChatModelErrorCode::Cancelled);
    tokio::time::timeout(Duration::from_secs(2), dropped.notified()).await.unwrap();
    assert_eq!(gate.calls.load(Ordering::Acquire), 0);
}

#[async_trait]
impl ProviderCredentialStore for PendingCredentialStore {
    async fn lease(
        &self,
        _: &ProviderCredentialRef,
        _: &CredentialTarget,
    ) -> Result<CredentialLease, ChatModelError> {
        let _guard = NotifyOnDrop(self.dropped.clone());
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_releases_pending_credential_acquisition() {
    let primary = route(ChatProtocol::Anthropic, "pending-credential", 1);
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(Notify::new());
    let transport = ScriptedTransport::new([]);
    let transports = transport_map([(ChatProtocol::Anthropic, provider_transport(&transport))]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet { primary: primary.clone(), failovers: vec![] },
        Arc::new(PendingCredentialStore { started: started.clone(), dropped: dropped.clone() }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let cancellation = CancellationToken::new();
    let _output = broker.open_chat_stream_cancellable(basic_request(&primary), cancellation.clone())
        .await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified()).await.unwrap();
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), dropped.notified()).await.unwrap();
    assert_eq!(transport.calls(), 0);
}

#[tokio::test]
async fn causality_and_credential_failures_stop_before_provider_transport() {
    let primary_route = route(ChatProtocol::Anthropic, "authority", 1);
    let request = basic_request(&primary_route);
    let transport = ScriptedTransport::new([TransportScript::Frames(successful_frames(
        "unexpected",
        "must not run",
    ))]);
    let transports = transport_map([(
        ChatProtocol::Anthropic,
        provider_transport(&transport),
    )]);
    let rejected = broker(
        StaticCausalityGate::reject(ChatModelError::new(
            ChatModelErrorCode::ShadowNotPrimary,
            "shadow requests cannot invoke the model",
            ChatRetryDirective::Never,
        )),
        ResolvedChatRouteSet {
            primary: primary_route.clone(),
            failovers: Vec::new(),
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let error = match rejected.open_stream(request.clone()).await {
        Ok(_) => panic!("causality rejection must happen before stream creation"),
        Err(error) => error,
    };
    assert_eq!(error.code, ChatModelErrorCode::ShadowNotPrimary);
    assert_eq!(transport.calls(), 0);

    let credential_mismatch = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: primary_route,
            failovers: Vec::new(),
        },
        Arc::new(StaticCredentialStore { mismatch: true }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let output = credential_mismatch
        .open_stream(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        output.last().unwrap().as_ref().unwrap_err().code,
        ChatModelErrorCode::CredentialTargetMismatch
    );
    assert_eq!(transport.calls(), 0);
}

#[tokio::test]
async fn stateless_responses_bridge_maps_broker_stream_and_rejects_storage() {
    let primary_route = route(ChatProtocol::OpenaiResponses, "bridge-route", 9);
    let transport = ScriptedTransport::new([TransportScript::Frames(vec![
        frame(
            "response.start",
            serde_json::json!({"id": "bridge-provider-response"}),
        ),
        frame("text.delta", serde_json::json!({"text": "bridge text"})),
        frame(
            "response.output_audio.delta",
            serde_json::json!({
                "media_type": "audio/pcm",
                "data_base64": "AAECAw=="
            }),
        ),
        frame(
            "usage",
            serde_json::json!({"input_tokens": 4, "output_tokens": 2}),
        ),
        frame("done", serde_json::json!({"finish_reason": "stop"})),
    ])]);
    let transports = transport_map([(
        ChatProtocol::OpenaiResponses,
        provider_transport(&transport),
    )]);
    let broker = broker(
        StaticCausalityGate::allow(),
        ResolvedChatRouteSet {
            primary: primary_route.clone(),
            failovers: Vec::new(),
        },
        Arc::new(StaticCredentialStore { mismatch: false }),
        &transports,
        BrokerRetryPolicy::default(),
    );
    let broker_port: Arc<dyn ChatBrokerPort> = broker;
    let bridge = ResponsesBridge::new(broker_port);
    let mut request = responses_request(&primary_route);
    request
        .requested_output_modalities
        .insert(ChatModality::Audio);
    let events = bridge
        .open_stream(request.clone())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(bridge.retry_count(), 0);
    assert!(matches!(
        events.first(),
        Some(ResponsesBridgeEvent::ResponseCreated { .. })
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesBridgeEvent::OutputTextDelta { delta, .. } if delta == "bridge text"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesBridgeEvent::OutputAudioDelta {
            media_type,
            data_base64,
            ..
        } if media_type == "audio/pcm" && data_base64 == "AAECAw=="
    )));
    assert!(matches!(
        events.last(),
        Some(ResponsesBridgeEvent::Completed {
            finish_reason: ChatFinishReason::Completed,
            ..
        })
    ));

    request.store = true;
    let error = match bridge.open_stream(request).await {
        Ok(_) => panic!("stateless bridge must reject store=true"),
        Err(error) => error,
    };
    assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
    assert_eq!(transport.calls(), 1);
}

#[test]
fn responses_bridge_schema_has_no_credential_surface() {
    let route = route(ChatProtocol::OpenaiChat, "bridge-schema", 1);
    let request = responses_request(&route);
    let mut value = serde_json::to_value(request).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("api_key".to_owned(), serde_json::json!("forbidden"));
    assert!(serde_json::from_value::<ResponsesBridgeRequest>(value).is_err());

    let mut metadata_request = responses_request(&route);
    metadata_request
        .metadata
        .insert("api_key".to_owned(), "forbidden".to_owned());
    let error = match metadata_request.into_chat_request() {
        Ok(_) => panic!("structured credential metadata must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
}

fn responses_request(route: &ResolvedChatRoute) -> ResponsesBridgeRequest {
    let chat = basic_request(route);
    ResponsesBridgeRequest {
        bridge_version: VersionString("chat-model-v1".to_owned()),
        causality: chat.causality,
        model_route_id: route.model_route_id.clone(),
        model_route_revision: route.model_route_revision,
        instructions: vec!["Answer through the stateless bridge.".to_owned()],
        input: vec![ResponsesInputItem::Message {
            role: ResponsesRole::User,
            content: vec![ResponsesInputContent::InputText {
                text: "Hello".to_owned(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ChatToolChoice::None,
        max_output_tokens: Some(128),
        reasoning: None,
        prompt_cache: PromptCachePolicy::Disabled,
        response_format: ChatResponseFormat::Text,
        requested_output_modalities: BTreeSet::from([ChatModality::Text]),
        previous_response_id: None,
        preserve_native_responses_items: false,
        metadata: BTreeMap::new(),
        store: false,
    }
}

#[tokio::test]
async fn argument_rejection_is_semantic_output_and_never_authorizes_broker_retry() {
    for terminal in [true, false] {
        let primary = route(ChatProtocol::OpenaiChat, "argument-rejection", 1);
        let mut frames = vec![frame("tool_call.completed", serde_json::json!({
            "call_id":"bad","name":"write_file","arguments":"{\"PRIVATE\":1}}"
        }))];
        if terminal { frames.push(frame("finish", serde_json::json!({"finish_reason":"tool_calls"}))); }
        let transport = ScriptedTransport::new([TransportScript::Frames(frames)]);
        let transports = transport_map([(ChatProtocol::OpenaiChat, provider_transport(&transport))]);
        let broker = broker(StaticCausalityGate::allow(), ResolvedChatRouteSet {
            primary: primary.clone(), failovers: Vec::new(),
        }, Arc::new(StaticCredentialStore { mismatch: false }), &transports, BrokerRetryPolicy::default());
        let events = broker.open_stream(basic_request(&primary)).await.unwrap().collect::<Vec<_>>().await;
        assert!(events.iter().any(|event| matches!(event,
            Ok(envelope) if matches!(&envelope.event, ChatModelEvent::ToolCallRejected { call_id, .. } if call_id.as_ref()=="bad"))));
        assert_eq!(transport.calls(), 1);
        if terminal {
            assert!(events.iter().all(Result::is_ok));
            assert!(matches!(events.last(), Some(Ok(envelope)) if matches!(envelope.event,
                ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })));
        } else {
            assert!(matches!(events.last(), Some(Err(error))
                if error.code==ChatModelErrorCode::StreamInterrupted && error.semantic_output_committed));
        }
    }
}

#[tokio::test]
async fn bridge_keeps_argument_rejection_separate_from_provider_failure() {
    let primary = route(ChatProtocol::OpenaiResponses, "bridge-rejected-call", 1);
    let transport = ScriptedTransport::new([TransportScript::Frames(vec![
        frame("tool_call.completed", serde_json::json!({"call_id":"bad","name":"write_file","arguments":"[]"})),
        frame("finish", serde_json::json!({"finish_reason":"tool_calls"})),
    ])]);
    let transports = transport_map([(ChatProtocol::OpenaiResponses, provider_transport(&transport))]);
    let broker = broker(StaticCausalityGate::allow(), ResolvedChatRouteSet {
        primary: primary.clone(), failovers: Vec::new(),
    }, Arc::new(StaticCredentialStore { mismatch: false }), &transports, BrokerRetryPolicy::default());
    let bridge = ResponsesBridge::new(broker);
    let events = bridge.open_stream(responses_request(&primary)).await.unwrap().collect::<Vec<_>>().await;
    assert!(events.iter().any(|event| matches!(event, ResponsesBridgeEvent::FunctionCallRejected {
        call_id, name, diagnostic, ..
    } if call_id.as_ref()=="bad" && name=="write_file" && diagnostic.category==nomifun_chat_model_broker::ToolArgumentDiagnosticCategory::RootType)));
    assert!(!events.iter().any(|event| matches!(event, ResponsesBridgeEvent::Failed { .. })));
    assert!(matches!(events.last(), Some(ResponsesBridgeEvent::Completed { finish_reason: ChatFinishReason::ToolCalls, .. })));
}
