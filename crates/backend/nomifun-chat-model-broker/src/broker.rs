use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use nomifun_agent_contracts::{ConnectionConfigRef, DigestHex, ModelRouteId};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::adapter::{ChatProtocolAdapter, ProviderWireStream};
use crate::contracts::{
    ChatContractError, ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatModelRequest, ChatProtocol,
    ChatRetryDirective, ChatToolCall, ResolvedChatRoute, ToolCallId,
};
use crate::ports::{
    ChatCapabilityObserver, ChatCausalityGate, ChatRouteResolver, CredentialTarget,
    NoopChatCapabilityObserver, ProviderCredentialStore,
};
use crate::provider_reasoning::ProviderReasoningRoute;

const BROKER_STREAM_CAPACITY: usize = 64;
const MAX_BUFFERED_PRE_SEMANTIC_EVENTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerRetryPolicy {
    pub max_total_attempts: u8,
    pub max_attempts_per_route: u8,
}

impl Default for BrokerRetryPolicy {
    fn default() -> Self {
        Self {
            max_total_attempts: 4,
            max_attempts_per_route: 3,
        }
    }
}

impl BrokerRetryPolicy {
    pub fn validate(self) -> Result<Self, ChatModelError> {
        if self.max_total_attempts == 0 || self.max_attempts_per_route == 0 {
            return Err(ChatModelError::invalid_request(
                "broker retry limits must be greater than zero",
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerEventEnvelope {
    pub route_id: ModelRouteId,
    pub route_revision: u64,
    pub protocol: ChatProtocol,
    pub provider_id: String,
    pub model: String,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
    pub route_attempt: u8,
    pub total_attempt: u8,
    pub event: ChatModelEvent,
}

pub struct ChatModelStream {
    receiver: mpsc::Receiver<Result<BrokerEventEnvelope, ChatModelError>>,
    cancellation: CancellationToken,
}

impl Drop for ChatModelStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Stream for ChatModelStream {
    type Item = Result<BrokerEventEnvelope, ChatModelError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.cancellation.is_cancelled() {
            self.receiver.close();
            return Poll::Ready(None);
        }
        self.receiver.poll_recv(context)
    }
}

#[async_trait]
pub trait ChatBrokerPort: Send + Sync {
    async fn open_chat_stream(
        &self,
        request: crate::contracts::ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError>;

    /// Process-local cancellation covering route preparation, credential
    /// acquisition, provider opening, streaming and backpressure. Implementors
    /// must drop the actual attempt, not only stop forwarding its output.
    /// Older custom ports fail closed until they implement this contract.
    async fn open_chat_stream_cancellable(
        &self,
        _request: crate::contracts::ChatModelRequest,
        _cancellation: CancellationToken,
    ) -> Result<ChatModelStream, ChatModelError> {
        Err(ChatModelError::new(
            ChatModelErrorCode::AdapterUnavailable,
            "chat broker does not support native attempt cancellation",
            ChatRetryDirective::Never,
        ))
    }
}

pub struct ChatModelBroker {
    causality_gate: Arc<dyn ChatCausalityGate>,
    route_resolver: Arc<dyn ChatRouteResolver>,
    credential_store: Arc<dyn ProviderCredentialStore>,
    adapters: BTreeMap<ChatProtocol, Arc<dyn ChatProtocolAdapter>>,
    retry_policy: BrokerRetryPolicy,
    capability_observer: Arc<dyn ChatCapabilityObserver>,
}

impl ChatModelBroker {
    pub fn new(
        causality_gate: Arc<dyn ChatCausalityGate>,
        route_resolver: Arc<dyn ChatRouteResolver>,
        credential_store: Arc<dyn ProviderCredentialStore>,
        adapters: impl IntoIterator<Item = Arc<dyn ChatProtocolAdapter>>,
        retry_policy: BrokerRetryPolicy,
    ) -> Result<Self, ChatModelError> {
        Self::new_with_capability_observer(
            causality_gate,
            route_resolver,
            credential_store,
            adapters,
            retry_policy,
            Arc::new(NoopChatCapabilityObserver),
        )
    }

    pub fn new_with_capability_observer(
        causality_gate: Arc<dyn ChatCausalityGate>,
        route_resolver: Arc<dyn ChatRouteResolver>,
        credential_store: Arc<dyn ProviderCredentialStore>,
        adapters: impl IntoIterator<Item = Arc<dyn ChatProtocolAdapter>>,
        retry_policy: BrokerRetryPolicy,
        capability_observer: Arc<dyn ChatCapabilityObserver>,
    ) -> Result<Self, ChatModelError> {
        let retry_policy = retry_policy.validate()?;
        let mut by_protocol = BTreeMap::new();
        for adapter in adapters {
            if adapter.retry_count() != 0 {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::ProtocolViolation,
                    format!(
                        "adapter {} declares autonomous retry; ChatModelBroker must be the sole retry owner",
                        adapter.name()
                    ),
                    ChatRetryDirective::Never,
                ));
            }
            if by_protocol.insert(adapter.protocol(), adapter).is_some() {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::ProtocolViolation,
                    "duplicate chat protocol adapter",
                    ChatRetryDirective::Never,
                ));
            }
        }
        let actual = by_protocol.keys().copied().collect::<BTreeSet<_>>();
        let expected = ChatProtocol::ALL.into_iter().collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(ChatModelError::new(
                ChatModelErrorCode::AdapterUnavailable,
                "ChatModelBroker requires the exact six-protocol adapter set",
                ChatRetryDirective::Never,
            ));
        }
        Ok(Self {
            causality_gate,
            route_resolver,
            credential_store,
            adapters: by_protocol,
            retry_policy,
            capability_observer,
        })
    }

    pub fn adapter_protocols(&self) -> BTreeSet<ChatProtocol> {
        self.adapters.keys().copied().collect()
    }

    async fn prepare(
        &self,
        request: &crate::contracts::ChatModelRequest,
    ) -> Result<Vec<ResolvedChatRoute>, ChatModelError> {
        request
            .validate()
            .map_err(contract_error_to_model_error)?;
        let routes = self.route_resolver.resolve(&request.route).await?;
        routes
            .validate_for(&request.route)
            .map_err(contract_error_to_model_error)?;
        let required = request.input.required_features();
        let mut candidates = Vec::new();
        for route in routes.candidates() {
            let Some(adapter) = self.adapters.get(&route.protocol) else {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::AdapterUnavailable,
                    format!("no adapter registered for {:?}", route.protocol),
                    ChatRetryDirective::Never,
                )
                .with_route(route.model_route_id.clone()));
            };
            if !required.is_superset(&route.activation_features)
                || !route.features.is_superset(&required)
                || !adapter.features().is_superset(&required)
                || request.input.messages.iter().any(|message| message.content.iter().any(|part|
                    matches!(part, crate::ChatContentPart::ProviderReasoning { block } if !block.matches_route(route))))
            {
                continue;
            }
            candidates.push(route.clone());
        }
        if candidates.is_empty() {
            return Err(ChatModelError::new(
                ChatModelErrorCode::UnsupportedFeature,
                "no resolved chat route can express the request without semantic loss",
                ChatRetryDirective::Never,
            ));
        }
        // Validate route identity and feature support before the gate claims
        // the operation. An unusable route plan must not permanently consume
        // an operation id in the Session facts.
        self.causality_gate.authorize(&request.causality).await?;
        Ok(candidates)
    }

    pub async fn open_stream(
        &self,
        request: crate::contracts::ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError> {
        self.open_stream_cancellable(request, CancellationToken::new()).await
    }

    pub async fn open_stream_cancellable(
        &self,
        request: crate::contracts::ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<ChatModelStream, ChatModelError> {
        // Dropping one request must not cancel its parent Session or siblings.
        let cancellation = cancellation.child_token();
        let routes = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ChatModelError::new(
                ChatModelErrorCode::Cancelled,
                "chat request was cancelled before opening the provider attempt",
                ChatRetryDirective::Never,
            )),
            result = self.prepare(&request) => result?,
        };
        let adapters = self.adapters.clone();
        let credential_store = Arc::clone(&self.credential_store);
        let retry_policy = self.retry_policy;
        let capability_observer = Arc::clone(&self.capability_observer);
        let (sender, receiver) = mpsc::channel(BROKER_STREAM_CAPACITY);

        let attempt_cancellation = cancellation.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = attempt_cancellation.cancelled() => {},
                _ = sender.closed() => {},
                _ = run_broker(
                    request,
                    routes,
                    adapters,
                    credential_store,
                    retry_policy,
                    capability_observer,
                    sender.clone(),
                ) => {},
            }
        });
        Ok(ChatModelStream { receiver, cancellation })
    }
}

#[async_trait]
impl ChatBrokerPort for ChatModelBroker {
    async fn open_chat_stream(
        &self,
        request: crate::contracts::ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError> {
        self.open_stream(request).await
    }

    async fn open_chat_stream_cancellable(
        &self,
        request: crate::contracts::ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<ChatModelStream, ChatModelError> {
        self.open_stream_cancellable(request, cancellation).await
    }
}

async fn run_broker(
    request: crate::contracts::ChatModelRequest,
    routes: Vec<ResolvedChatRoute>,
    adapters: BTreeMap<ChatProtocol, Arc<dyn ChatProtocolAdapter>>,
    credential_store: Arc<dyn ProviderCredentialStore>,
    retry_policy: BrokerRetryPolicy,
    capability_observer: Arc<dyn ChatCapabilityObserver>,
    sender: mpsc::Sender<Result<BrokerEventEnvelope, ChatModelError>>,
) {
    let mut route_index = 0_usize;
    let mut route_attempt = 0_u8;
    let mut total_attempt = 0_u8;
    let mut last_error = None;

    while route_index < routes.len() && total_attempt < retry_policy.max_total_attempts {
        if sender.is_closed() {
            return;
        }
        let route = &routes[route_index];
        route_attempt = route_attempt.saturating_add(1);
        total_attempt = total_attempt.saturating_add(1);

        tracing::debug!(
            route_id = route.model_route_id.as_ref(),
            route_revision = route.model_route_revision,
            protocol = ?route.protocol,
            provider_id = route.provider_id.as_ref(),
            route_attempt,
            total_attempt,
            "ChatModelBroker opening one provider attempt"
        );

        let outcome = run_attempt(
            &request,
            route,
            adapters
                .get(&route.protocol)
                .expect("validated protocol adapter")
                .as_ref(),
            credential_store.as_ref(),
            route_attempt,
            total_attempt,
            &sender,
        )
        .await;

        match outcome {
            AttemptOutcome::Completed | AttemptOutcome::ReceiverDropped => return,
            AttemptOutcome::Failed {
                error,
                semantic_output_committed: true,
            } => {
                let _ = sender.send(Err(error.after_semantic_output())).await;
                return;
            }
            AttemptOutcome::Failed {
                error,
                semantic_output_committed: false,
            } => {
                if let Some(feature) = error.unsupported_feature {
                    capability_observer.record_unsupported(route, feature).await;
                }
                let retry = error.retry;
                let delay = crate::retry::delay(&error, total_attempt);
                last_error = Some(error);
                let total_capacity =
                    total_attempt < retry_policy.max_total_attempts;
                let Some(delay) = delay.filter(|_| total_capacity) else { break; };
                if retry == ChatRetryDirective::RetrySameRoute
                    && route_attempt < retry_policy.max_attempts_per_route
                {
                    // Keep this exact admitted route and lease fresh credentials
                    // for its next attempt only after the cooldown has elapsed.
                } else if retry != ChatRetryDirective::Never
                    && route_index + 1 < routes.len()
                {
                    route_index += 1;
                    route_attempt = 0;
                } else {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = sender.closed() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }

    let error = last_error.unwrap_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::ProviderUnavailable,
            "all deterministic chat model route attempts were exhausted",
            ChatRetryDirective::Never,
        )
    });
    let _ = sender.send(Err(error)).await;
}

async fn run_attempt(
    request: &crate::contracts::ChatModelRequest,
    route: &ResolvedChatRoute,
    adapter: &dyn ChatProtocolAdapter,
    credential_store: &dyn ProviderCredentialStore,
    route_attempt: u8,
    total_attempt: u8,
    sender: &mpsc::Sender<Result<BrokerEventEnvelope, ChatModelError>>,
) -> AttemptOutcome {
    let target = CredentialTarget::for_route(route);
    let credential = match credential_store
        .lease(&route.credential_ref, &target)
        .await
    {
        Ok(credential) if credential.validates_route(route) => credential,
        Ok(_) => {
            return AttemptOutcome::failed(ChatModelError::new(
                ChatModelErrorCode::CredentialTargetMismatch,
                "credential store returned authority for a different route target",
                ChatRetryDirective::Never,
            )
            .with_route(route.model_route_id.clone()));
        }
        Err(error) => {
            return AttemptOutcome::failed(error.with_route(route.model_route_id.clone()));
        }
    };

    let wire_request = match adapter.encode_request(request, route, &credential) {
        Ok(request) => request,
        Err(error) => {
            return AttemptOutcome::failed(error.with_route(route.model_route_id.clone()));
        }
    };
    let wire_stream = match adapter.open_stream(wire_request, credential).await {
        Ok(stream) => stream,
        Err(error) => {
            return AttemptOutcome::failed(error.with_route(route.model_route_id.clone()));
        }
    };

    consume_attempt_stream(
        route,
        adapter,
        request,
        wire_stream,
        route_attempt,
        total_attempt,
        sender,
    )
    .await
}

async fn consume_attempt_stream(
    route: &ResolvedChatRoute,
    adapter: &dyn ChatProtocolAdapter,
    request: &ChatModelRequest,
    mut wire_stream: ProviderWireStream,
    route_attempt: u8,
    total_attempt: u8,
    sender: &mpsc::Sender<Result<BrokerEventEnvelope, ChatModelError>>,
) -> AttemptOutcome {
    let mut sequence = EventSequence::default();
    let mut buffered = Vec::new();
    let mut semantic_output_committed = false;
    let mut decoder = adapter.new_frame_decoder_for(request);

    loop {
        let frame = tokio::select! {
            biased;
            _ = sender.closed() => return AttemptOutcome::ReceiverDropped,
            frame = wire_stream.next() => frame,
        };
        let Some(frame) = frame else { break; };
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                return AttemptOutcome::Failed {
                    error: error.with_route(route.model_route_id.clone()),
                    semantic_output_committed,
                };
            }
        };
        let decoded = match decoder.as_mut() {
            Some(decoder) => decoder.decode_frame(frame),
            None => adapter.decode_frame(frame),
        };
        let events = match decoded {
            Ok(events) => events,
            Err(error) => {
                return AttemptOutcome::Failed {
                    error: error.with_route(route.model_route_id.clone()),
                    semantic_output_committed,
                };
            }
        };

        for mut event in events {
            if let ChatModelEvent::ProviderReasoningBlock { block } = &mut event {
                if let Err(message) = block.bind_route(route) {
                    return AttemptOutcome::Failed {
                        error: ChatModelError::protocol_violation(message).with_route(route.model_route_id.clone()),
                        semantic_output_committed,
                    };
                }
            }
            if let Err(error) = sequence.observe(&event) {
                return AttemptOutcome::Failed {
                    error: error.with_route(route.model_route_id.clone()),
                    semantic_output_committed,
                };
            }

            let terminal = event.is_terminal();
            let semantic = event.is_semantic_output();
            let envelope = envelope_for(
                route,
                route_attempt,
                total_attempt,
                event,
            );
            if !semantic_output_committed && !semantic {
                if buffered.len() >= MAX_BUFFERED_PRE_SEMANTIC_EVENTS {
                    return AttemptOutcome::failed(
                        ChatModelError::protocol_violation(
                            "provider emitted too many pre-semantic stream events",
                        )
                        .with_route(route.model_route_id.clone()),
                    );
                }
                buffered.push(envelope);
                continue;
            }

            if !semantic_output_committed {
                semantic_output_committed = true;
                for buffered_event in buffered.drain(..) {
                    if sender.send(Ok(buffered_event)).await.is_err() {
                        return AttemptOutcome::ReceiverDropped;
                    }
                }
            }
            if sender.send(Ok(envelope)).await.is_err() {
                return AttemptOutcome::ReceiverDropped;
            }
            if terminal {
                return AttemptOutcome::Completed;
            }
        }
    }

    AttemptOutcome::Failed {
        error: ChatModelError::stream_interrupted(
            "provider stream ended before a canonical terminal event",
        )
        .with_route(route.model_route_id.clone()),
        semantic_output_committed,
    }
}

fn envelope_for(
    route: &ResolvedChatRoute,
    route_attempt: u8,
    total_attempt: u8,
    event: ChatModelEvent,
) -> BrokerEventEnvelope {
    BrokerEventEnvelope {
        route_id: route.model_route_id.clone(),
        route_revision: route.model_route_revision,
        protocol: route.protocol,
        provider_id: route.provider_id.as_ref().to_owned(),
        model: route.model.clone(),
        connection_config_ref: route.connection_config_ref.clone(),
        config_revision_digest: route.config_revision_digest.clone(),
        route_attempt,
        total_attempt,
        event,
    }
}

enum AttemptOutcome {
    Completed,
    ReceiverDropped,
    Failed {
        error: ChatModelError,
        semantic_output_committed: bool,
    },
}

impl AttemptOutcome {
    fn failed(error: ChatModelError) -> Self {
        Self::Failed {
            error,
            semantic_output_committed: false,
        }
    }
}

#[derive(Default)]
struct EventSequence {
    response_started: bool,
    usage_seen: bool,
    terminal_seen: bool,
    tool_names: BTreeMap<ToolCallId, String>,
}

impl EventSequence {
    fn observe(&mut self, event: &ChatModelEvent) -> Result<(), ChatModelError> {
        if self.terminal_seen {
            return Err(ChatModelError::protocol_violation(
                "provider emitted an event after the canonical terminal",
            ));
        }
        match event {
            ChatModelEvent::ResponseStarted { .. } => {
                if self.response_started {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted response_started more than once",
                    ));
                }
                self.response_started = true;
            }
            ChatModelEvent::ToolCallDelta {
                call_id,
                name,
                arguments_delta: _,
            } => {
                validate_tool_identity(call_id, name, &mut self.tool_names)?;
            }
            ChatModelEvent::ToolCallCompleted { call } => {
                validate_completed_tool(call, &mut self.tool_names)?;
            }
            ChatModelEvent::Usage { .. } => {
                if self.usage_seen {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted canonical usage more than once",
                    ));
                }
                self.usage_seen = true;
            }
            ChatModelEvent::Completed { .. } => {
                self.terminal_seen = true;
            }
            ChatModelEvent::OutputTextDelta { text }
            | ChatModelEvent::ReasoningDelta { text } => {
                if text.is_empty() {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted an empty semantic text delta",
                    ));
                }
            }
            ChatModelEvent::OutputAudioDelta {
                media_type,
                data_base64,
            } => {
                if media_type.trim().is_empty() || data_base64.trim().is_empty() {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted an invalid semantic audio delta",
                    ));
                }
            }
            ChatModelEvent::ReasoningSignature { signature } => {
                if signature.is_empty() {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted an empty reasoning signature",
                    ));
                }
            }
            ChatModelEvent::ReasoningBlock { text, encrypted_content } => {
                if encrypted_content.as_ref().is_some_and(String::is_empty)
                    || (text.is_empty() && encrypted_content.is_none())
                {
                    return Err(ChatModelError::protocol_violation("provider emitted an empty reasoning block"));
                }
            }
            ChatModelEvent::ProviderReasoningBlock { block } => {
                block.validate().map_err(ChatModelError::protocol_violation)?;
            }
            ChatModelEvent::ProviderRoundId { round_id } => {
                if round_id.as_ref().is_empty() {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted an empty round id",
                    ));
                }
            }
            ChatModelEvent::NativeResponsesItem { item_type, .. } => {
                if item_type.trim().is_empty() {
                    return Err(ChatModelError::protocol_violation(
                        "provider emitted an untyped native Responses item",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_tool_identity(
    call_id: &ToolCallId,
    name: &str,
    tool_names: &mut BTreeMap<ToolCallId, String>,
) -> Result<(), ChatModelError> {
    if call_id.as_ref().trim().is_empty() {
        return Err(ChatModelError::protocol_violation(
            "tool-call event has an empty call id",
        ));
    }
    if let Some(existing) = tool_names.get(call_id) {
        if !name.is_empty() && existing != name {
            return Err(ChatModelError::protocol_violation(
                "tool-call name changed for one call id",
            ));
        }
    } else {
        if name.trim().is_empty() {
            return Err(ChatModelError::protocol_violation(
                "first tool-call event has an empty name",
            ));
        }
        tool_names.insert(call_id.clone(), name.to_owned());
    }
    Ok(())
}

fn validate_completed_tool(
    call: &ChatToolCall,
    tool_names: &mut BTreeMap<ToolCallId, String>,
) -> Result<(), ChatModelError> {
    call.validate()
        .map_err(|error| ChatModelError::protocol_violation(error.to_string()))?;
    validate_tool_identity(&call.call_id, &call.name, tool_names)
}

fn contract_error_to_model_error(error: ChatContractError) -> ChatModelError {
    ChatModelError::invalid_request(error.to_string())
}
