use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use nomifun_agent_contracts::{
    AgentSessionId, ConnectionConfigRef, DigestHex, EventId, ModelRouteId, OperationId,
    ResolvedSnapshotId, ResolvedSnapshotRef, VersionString,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatBrokerPort, ChatCausality,
    ChatCausalityGate, ChatContentPart, ChatFinishReason, ChatMessage, ChatModelBroker,
    ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatModelInput, ChatModelRequest,
    ChatModality, ChatProtocol, ChatProtocolAdapter, ChatResponseFormat, ChatRetryDirective,
    ChatRole, ChatRouteResolver, ChatRouteSelection, ChatTask, ChatToolChoice,
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
        assert_eq!(
            credential.target().model_route_id,
            request.model_route_id
        );
        assert_eq!(
            credential.target().model_route_revision,
            request.model_route_revision
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
            model_route_revision: route.model_route_revision,
            operation_id: OperationId("model-operation-test".to_owned()),
        },
        route: ChatRouteSelection {
            model_route_id: route.model_route_id.clone(),
            model_route_revision: route.model_route_revision,
            task: ChatTask::AgentChat,
        },
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
        assert_eq!(request.model_route_id, fixture.route.model_route_id);
        assert_eq!(
            request.model_route_revision,
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
