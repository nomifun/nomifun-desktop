use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use nomifun_agent_contracts::{
    AgentSessionId, ChatRouteIdentity, ConnectionConfigRef, DigestHex, EventId, ModelRouteId,
    OperationId, ResolvedSnapshotId, ResolvedSnapshotRef, VersionString,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatBrokerPort, ChatCausality,
    ChatCausalityGate, ChatContentPart, ChatFinishReason, ChatMessage, ChatModelBroker,
    ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatModelInput, ChatModelRequest,
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

enum TransportScript {
    OpenError(ChatModelError),
    Frames(Vec<Result<ProviderWireFrame, ChatModelError>>),
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
                text: "Hello ".to_owned()
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
                text: "complete me".to_owned()
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
            ChatModelEvent::ReasoningSignature {
                signature: "sig-1".to_owned()
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
                text: "done".to_owned()
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
            ChatModelEvent::OutputTextDelta { text } if text == "hello from failover"
        )
    }));
    assert!(
        events
            .last()
            .is_some_and(|event| event.event.is_terminal())
    );
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
            ChatModelEvent::OutputTextDelta { text } if text == "committed text"
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
