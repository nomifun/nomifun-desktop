//! Host-owned bridge from a Runtime `StartTurn` acknowledgement to the
//! provider ChatModelBroker and the canonical SessionEvent projections.
//!
//! The Codex sidecar owns its Runtime protocol, while NomiFun owns provider
//! routing and Session facts.  This module is the small composition boundary
//! between the two: the sidecar acknowledges `StartTurn`, then the host builds
//! one stateless Responses request from the already committed turn input,
//! opens the injected broker stream, and persists the bounded result.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use nomifun_agent_contracts::{
    AgentSessionId, ChatRouteIdentity, CorrelationId, EventId, EventProducerId, IdempotencyKey,
    OperationId, RuntimeBindingContract, RuntimeBindingId, RuntimeCommand,
    RuntimeSessionDisposeParams, RuntimeStartTurnParams, SemanticSessionEventDraft,
    SessionEventAppend, SessionEventKind, SessionEventPayloadRef, SessionEventRecord,
    StrictJsonValue, canonical_json_bytes,
};
use nomifun_agent_session::{AgentSessionStore, ChatCausalityFacts, SessionStoreError};
use nomifun_chat_model_broker::{
    ChatBrokerPort, ChatCausality, ChatFinishReason, ChatModelError, ChatModelErrorCode,
    ChatModality, ChatResponseFormat, ChatRetryDirective, ChatToolChoice, ResponsesBridge,
    ResponsesBridgeEvent, ResponsesBridgeRequest,
    ResponsesInputContent, ResponsesInputItem, CHAT_MODEL_CONTRACT_VERSION, ChatUsage,
    ProviderResponseId, PromptCachePolicy,
};
use nomifun_codex_runtime::{
    ManagedRuntimeSession, RuntimeDisposeReport, RuntimeError, RuntimeLaunchRequest,
};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::platform::CodexRuntimePort;

const RUNTIME_SUPERVISOR: &str = "runtime_supervisor";
const MAX_CONTEXT_INSTRUCTION_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum RuntimeChatBridgeError {
    #[error("runtime StartTurn admission rejected: {0}")]
    Admission(&'static str),
    #[error("AgentSession facts unavailable: {0}")]
    Session(#[from] SessionStoreError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeTurnTerminal {
    Completed {
        event_id: EventId,
        finish_reason: ChatFinishReason,
    },
    Failed {
        event_id: EventId,
        code: ChatModelErrorCode,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTurnResult {
    pub agent_session_id: AgentSessionId,
    pub turn_operation_id: OperationId,
    pub response_id: Option<ProviderResponseId>,
    pub message_event_id: Option<EventId>,
    pub content: String,
    pub part_count: u64,
    pub terminal: RuntimeTurnTerminal,
}

#[derive(Clone)]
pub struct RuntimeStartTurnBrokerBridge {
    sessions: Arc<AgentSessionStore>,
    broker: Arc<dyn ChatBrokerPort>,
}

impl RuntimeStartTurnBrokerBridge {
    pub fn new(sessions: Arc<AgentSessionStore>, broker: Arc<dyn ChatBrokerPort>) -> Self {
        Self { sessions, broker }
    }

    /// Run one host-owned text turn to a durable terminal event.
    ///
    /// Provider failures are represented by `turn/failed` and returned as a
    /// successful bridge result.  Only admission or persistence failures
    /// escape as `Err`, because those mean the host could not establish the
    /// canonical terminal fact.
    pub async fn run(
        &self,
        params: RuntimeStartTurnParams,
    ) -> Result<RuntimeTurnResult, RuntimeChatBridgeError> {
        let admitted = match self.admit(&params).await {
            Ok(admitted) => admitted,
            Err(error) => {
                self.record_admission_failure(&params).await;
                return Err(error);
            }
        };
        let mut state = ProjectionState {
            message_correlation: CorrelationId::from(format!(
                "runtime-chat-message:{}",
                admitted.turn_operation_id.as_ref()
            )),
            last_causation_event_id: admitted.turn_event_id.clone(),
            response_id: None,
            content: String::new(),
            part_count: 0,
            usage: None,
        };
        let request = responses_request(&params, &admitted);
        let mut stream = match ResponsesBridge::new(Arc::clone(&self.broker))
            .open_stream(request)
            .await
        {
            Ok(stream) => stream,
            Err(error) if error.code == ChatModelErrorCode::DuplicateOperation => {
                return Err(RuntimeChatBridgeError::Admission(
                    "model operation was already projected",
                ));
            }
            Err(error) => return self.fail(&admitted, &state, error).await,
        };

        while let Some(event) = stream.next().await {
            match event {
                ResponsesBridgeEvent::ResponseCreated { response_id } => {
                    if state.response_id.replace(response_id).is_some() {
                        let error = ChatModelError::protocol_violation(
                            "Responses bridge emitted more than one response.created event",
                        );
                        return self.fail(&admitted, &state, error).await;
                    }
                }
                ResponsesBridgeEvent::OutputTextDelta { delta, .. } => {
                    self.ensure_active(&admitted).await?;
                    if delta.is_empty() {
                        let error = ChatModelError::protocol_violation(
                            "Responses bridge emitted an empty text delta",
                        );
                        return self.fail(&admitted, &state, error).await;
                    }
                    let record = self
                        .append_message_part(&admitted, &mut state, delta)
                        .await?;
                    state.last_causation_event_id = record.event_id;
                }
                ResponsesBridgeEvent::Usage { usage, .. } => {
                    state.usage = Some(usage);
                }
                ResponsesBridgeEvent::Completed {
                    finish_reason, ..
                } => {
                    self.ensure_active(&admitted).await?;
                    return self.complete(&admitted, &state, finish_reason).await;
                }
                ResponsesBridgeEvent::Failed { error, .. } => {
                    return self.fail(&admitted, &state, error).await;
                }
                ResponsesBridgeEvent::OutputAudioDelta { .. }
                | ResponsesBridgeEvent::ReasoningDelta { .. }
                | ResponsesBridgeEvent::ReasoningSignature { .. }
                | ResponsesBridgeEvent::FunctionCallArgumentsDelta { .. }
                | ResponsesBridgeEvent::FunctionCallDone { .. }
                | ResponsesBridgeEvent::NativeItem { .. }
                | ResponsesBridgeEvent::ProviderRound { .. } => {
                    let error = ChatModelError::new(
                        ChatModelErrorCode::UnsupportedFeature,
                        "runtime bridge only projects text Responses output",
                        ChatRetryDirective::Never,
                    );
                    return self.fail(&admitted, &state, error).await;
                }
            }
        }

        let error = ChatModelError::new(
            ChatModelErrorCode::StreamInterrupted,
            "Responses bridge ended without a terminal event",
            ChatRetryDirective::Never,
        );
        self.fail(&admitted, &state, error).await
    }

    async fn record_admission_failure(&self, params: &RuntimeStartTurnParams) {
        let Ok(facts) = self
            .sessions
            .chat_causality_facts(
                &params.context.agent_session_id,
                &params.context.operation_id,
            )
            .await
        else {
            return;
        };
        if facts.head.status != "running"
            || facts.head.active_turn_id.as_deref()
                != Some(params.context.operation_id.as_ref())
        {
            return;
        }
        let Some(turn) = facts.events.iter().find(|event| {
            event.kind.0 == "turn/started"
                && event.correlation_id.as_ref() == params.context.operation_id.as_ref()
        }) else {
            return;
        };
        let append = SessionEventAppend {
            agent_session_id: params.context.agent_session_id.clone(),
            event_id: EventId::from(format!(
                "runtime-chat:turn-failed:{}",
                params.context.operation_id.as_ref()
            )),
            producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
            idempotency_key: IdempotencyKey::from(format!(
                "runtime-chat:turn-failed:{}",
                params.context.operation_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/failed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(
                    params.context.operation_id.as_ref().to_owned(),
                ),
                causation_event_id: Some(turn.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "code": ChatModelErrorCode::CausalityRejected,
                    "retry": ChatRetryDirective::Never,
                    "semantic_output_committed": false
                }))),
            },
        };
        if let Err(error) = self
            .sessions
            .append_turn_terminal(&append, &params.context.operation_id)
            .await
        {
            tracing::debug!(
                ?error,
                operation_id = params.context.operation_id.as_ref(),
                "runtime bridge admission failure did not own the turn terminal"
            );
        }
    }

    async fn admit(
        &self,
        params: &RuntimeStartTurnParams,
    ) -> Result<AdmittedTurn, RuntimeChatBridgeError> {
        let context = &params.context;
        let facts = self
            .sessions
            .chat_causality_facts(&context.agent_session_id, &context.operation_id)
            .await?;

        if facts.session.agent_session_id != context.agent_session_id {
            return Err(RuntimeChatBridgeError::Admission(
                "Session facts belong to another AgentSession",
            ));
        }
        if facts.head.status != "running"
            || facts.head.active_turn_id.as_deref() != Some(context.operation_id.as_ref())
        {
            return Err(RuntimeChatBridgeError::Admission(
                "Session is not at the active StartTurn boundary",
            ));
        }
        if facts.head.active_set_generation != context.active_set_generation {
            return Err(RuntimeChatBridgeError::Admission(
                "active capability generation differs from Runtime context",
            ));
        }
        if facts.session.agent_binding.resolved_snapshot_ref != context.resolved_snapshot_ref
            || facts.head.snapshot_digest.as_deref()
                != Some(context.resolved_snapshot_ref.snapshot_digest.as_ref())
        {
            return Err(RuntimeChatBridgeError::Admission(
                "Runtime context does not use the Session frozen Snapshot",
            ));
        }

        let runtime_bound = facts
            .events
            .iter()
            .find(|event| {
                event.kind.0 == "runtime/bound"
                    && event.runtime_binding_id.as_ref() == Some(&context.runtime_binding_id)
            })
            .ok_or(RuntimeChatBridgeError::Admission(
                "Runtime binding has no committed runtime/bound fact",
            ))?;
        if facts.head.runtime_bound_event_id.as_deref() != Some(runtime_bound.event_id.as_ref()) {
            return Err(RuntimeChatBridgeError::Admission(
                "Session head is bound to a different Runtime event",
            ));
        }
        let runtime_payload = payload_for(&facts, runtime_bound)?;
        if runtime_payload
            .get("runtime_profile_digest")
            .and_then(Value::as_str)
            != Some(context.runtime_profile_digest.as_ref())
        {
            return Err(RuntimeChatBridgeError::Admission(
                "Runtime profile digest differs from the committed binding",
            ));
        }

        let turn_event = facts
            .events
            .iter()
            .filter(|event| {
                event.kind.0 == "turn/started"
                    && event.correlation_id.as_ref() == context.operation_id.as_ref()
            })
            .max_by_key(|event| event.seq)
            .ok_or(RuntimeChatBridgeError::Admission(
                "active turn/started fact is missing",
            ))?;
        let turn_payload = payload_for(&facts, turn_event)?;
        if turn_payload
            .get("operation_id")
            .and_then(Value::as_str)
            != Some(context.operation_id.as_ref())
            || turn_payload
                .get("input_event_id")
                .and_then(Value::as_str)
                != Some(params.input_event_id.as_ref())
            || turn_event.causation_event_id.as_ref() != Some(&params.input_event_id)
        {
            return Err(RuntimeChatBridgeError::Admission(
                "StartTurn input is not the committed active turn cause",
            ));
        }

        let input_event = facts
            .events
            .iter()
            .find(|event| event.event_id == params.input_event_id)
            .ok_or(RuntimeChatBridgeError::Admission(
                "StartTurn input event is not committed",
            ))?;
        if input_event.kind.0 != "message/user-accepted"
            || input_event.idempotency_key != params.idempotency_key
        {
            return Err(RuntimeChatBridgeError::Admission(
                "StartTurn input event identity differs from the Runtime command",
            ));
        }
        let input_payload = payload_for(&facts, input_event)?;
        let input_text = input_payload
            .get("content")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(RuntimeChatBridgeError::Admission(
                "minimum Runtime bridge requires a non-empty text input",
            ))?
            .to_owned();

        let route_identity = turn_payload
            .get("route_identity")
            .cloned()
            .ok_or(RuntimeChatBridgeError::Admission(
                "active turn has no exact model route identity",
            ))
            .and_then(|value| {
                serde_json::from_value::<ChatRouteIdentity>(value).map_err(|_| {
                    RuntimeChatBridgeError::Admission(
                        "active turn route identity is not canonical",
                    )
                })
            })?;
        route_identity.validate().map_err(|_| {
            RuntimeChatBridgeError::Admission(
                "active turn route identity failed canonical validation",
            )
        })?;
        if facts.turn_route_identities != BTreeSet::from([route_identity.clone()]) {
            return Err(RuntimeChatBridgeError::Admission(
                "active turn carries more than one model route identity",
            ));
        }
        let instructions = context_instructions(turn_payload)?;

        Ok(AdmittedTurn {
            session_id: context.agent_session_id.clone(),
            turn_operation_id: context.operation_id.clone(),
            turn_event_id: turn_event.event_id.clone(),
            route_identity,
            input_text,
            instructions,
        })
    }

    async fn ensure_active(
        &self,
        admitted: &AdmittedTurn,
    ) -> Result<(), RuntimeChatBridgeError> {
        let head = self.sessions.head(&admitted.session_id).await?;
        if head.status != "running"
            || head.active_turn_id.as_deref() != Some(admitted.turn_operation_id.as_ref())
        {
            return Err(RuntimeChatBridgeError::Admission(
                "the active turn crossed a terminal or cancel fence",
            ));
        }
        Ok(())
    }

    async fn append_message_part(
        &self,
        admitted: &AdmittedTurn,
        state: &mut ProjectionState,
        delta: String,
    ) -> Result<SessionEventRecord, RuntimeChatBridgeError> {
        let index = state.part_count;
        let content = delta.clone();
        let event_id = EventId::from(format!(
            "runtime-chat:message-part:{}:{}",
            admitted.turn_operation_id.as_ref(),
            index
        ));
        let result = self
            .sessions
            .append_event(&SessionEventAppend {
                agent_session_id: admitted.session_id.clone(),
                event_id,
                producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
                idempotency_key: IdempotencyKey::from(format!(
                    "runtime-chat:message-part:{}:{}",
                    admitted.turn_operation_id.as_ref(),
                    index
                )),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("message/content-part".to_owned()),
                    kind_version: 1,
                    correlation_id: state.message_correlation.clone(),
                    causation_event_id: Some(state.last_causation_event_id.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                        json!({ "content": delta }),
                    )),
                },
        })
        .await?;
        let record = required_record(result, "message/content-part")?;
        state.content.push_str(&content);
        state.part_count = state.part_count.saturating_add(1);
        Ok(record)
    }

    async fn complete(
        &self,
        admitted: &AdmittedTurn,
        state: &ProjectionState,
        finish_reason: ChatFinishReason,
    ) -> Result<RuntimeTurnResult, RuntimeChatBridgeError> {
        let message_event_id = EventId::from(format!(
            "runtime-chat:message-completed:{}",
            admitted.turn_operation_id.as_ref()
        ));
        let mut message_payload = json!({
            "content_digest": nomifun_agent_contracts::digest_bytes(state.content.as_bytes()),
            "part_count": state.part_count,
        });
        if let Some(usage) = &state.usage {
            message_payload["usage"] = json!(usage);
        }
        if let Some(response_id) = &state.response_id {
            message_payload["response_id"] = json!(response_id);
        }
        let message_append = SessionEventAppend {
            agent_session_id: admitted.session_id.clone(),
            event_id: message_event_id,
            producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
            idempotency_key: IdempotencyKey::from(format!(
                "runtime-chat:message-completed:{}",
                admitted.turn_operation_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/completed".to_owned()),
                kind_version: 1,
                correlation_id: state.message_correlation.clone(),
                causation_event_id: Some(state.last_causation_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(message_payload)),
            },
        };
        let turn_append = SessionEventAppend {
            agent_session_id: admitted.session_id.clone(),
            event_id: EventId::from(format!(
                "runtime-chat:turn-completed:{}",
                admitted.turn_operation_id.as_ref()
            )),
            producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
            idempotency_key: IdempotencyKey::from(format!(
                "runtime-chat:turn-completed:{}",
                admitted.turn_operation_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/completed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(
                    admitted.turn_operation_id.as_ref().to_owned(),
                ),
                causation_event_id: Some(message_append.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "message_event_id": message_append.event_id,
                    "finish_reason": finish_reason,
                    "response_id": state.response_id
                }))),
            },
        };
        let (message_result, turn_result) = self
            .sessions
            .append_chat_completion(
                &message_append,
                &turn_append,
                &admitted.turn_operation_id,
            )
            .await?;
        let message = required_record(message_result, "message/completed")?;
        let turn = required_record(turn_result, "turn/completed")?;
        Ok(RuntimeTurnResult {
            agent_session_id: admitted.session_id.clone(),
            turn_operation_id: admitted.turn_operation_id.clone(),
            response_id: state.response_id.clone(),
            message_event_id: Some(message.event_id),
            content: state.content.clone(),
            part_count: state.part_count,
            terminal: RuntimeTurnTerminal::Completed {
                event_id: turn.event_id,
                finish_reason,
            },
        })
    }

    async fn fail(
        &self,
        admitted: &AdmittedTurn,
        state: &ProjectionState,
        error: ChatModelError,
    ) -> Result<RuntimeTurnResult, RuntimeChatBridgeError> {
        let event_id = EventId::from(format!(
            "runtime-chat:turn-failed:{}",
            admitted.turn_operation_id.as_ref()
        ));
        let result = self
            .sessions
            .append_turn_terminal(
                &SessionEventAppend {
                agent_session_id: admitted.session_id.clone(),
                event_id,
                producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
                idempotency_key: IdempotencyKey::from(format!(
                    "runtime-chat:turn-failed:{}",
                    admitted.turn_operation_id.as_ref()
                )),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("turn/failed".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(
                        admitted.turn_operation_id.as_ref().to_owned(),
                    ),
                    causation_event_id: Some(state.last_causation_event_id.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "code": error.code,
                        "retry": error.retry,
                        "semantic_output_committed": error.semantic_output_committed
                    }))),
                },
                },
                &admitted.turn_operation_id,
            )
            .await?;
        let record = required_record(result, "turn/failed")?;
        Ok(RuntimeTurnResult {
            agent_session_id: admitted.session_id.clone(),
            turn_operation_id: admitted.turn_operation_id.clone(),
            response_id: state.response_id.clone(),
            message_event_id: None,
            content: state.content.clone(),
            part_count: state.part_count,
            terminal: RuntimeTurnTerminal::Failed {
                event_id: record.event_id,
                code: error.code,
            },
        })
    }
}

struct AdmittedTurn {
    session_id: AgentSessionId,
    turn_operation_id: OperationId,
    turn_event_id: EventId,
    route_identity: ChatRouteIdentity,
    input_text: String,
    instructions: Vec<String>,
}

struct ProjectionState {
    message_correlation: CorrelationId,
    last_causation_event_id: EventId,
    response_id: Option<ProviderResponseId>,
    content: String,
    part_count: u64,
    usage: Option<ChatUsage>,
}

fn payload_for<'a>(
    facts: &'a ChatCausalityFacts,
    event: &SessionEventRecord,
) -> Result<&'a Value, RuntimeChatBridgeError> {
    facts
        .event_payloads
        .get(event.event_id.as_ref())
        .ok_or(RuntimeChatBridgeError::Admission(
            "committed event payload is unavailable",
        ))
}

fn required_record(
    result: nomifun_agent_session::SessionEventAppendResult,
    kind: &'static str,
) -> Result<SessionEventRecord, RuntimeChatBridgeError> {
    result.record.ok_or(RuntimeChatBridgeError::Session(
        SessionStoreError::InvalidEvent(format!("{kind} did not produce a persisted record")),
    ))
}

fn model_operation_id(turn_operation_id: &OperationId) -> OperationId {
    OperationId::from(format!("runtime-chat:model:{}", turn_operation_id.as_ref()))
}

fn context_instructions(payload: &Value) -> Result<Vec<String>, RuntimeChatBridgeError> {
    let Some(raw) = payload.get("context_contributions") else {
        return Ok(Vec::new());
    };
    let entries = raw.as_array().ok_or(RuntimeChatBridgeError::Admission(
        "turn context contributions are not an array",
    ))?;
    let mut total = 0usize;
    let mut instructions = Vec::with_capacity(entries.len());
    for entry in entries {
        let object = entry.as_object().ok_or(RuntimeChatBridgeError::Admission(
            "turn context contribution is not an object",
        ))?;
        let capability_id = object
            .get("capability_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or(RuntimeChatBridgeError::Admission(
                "turn context contribution has no capability identity",
            ))?;
        let value = object
            .get("value")
            .ok_or(RuntimeChatBridgeError::Admission(
                "turn context contribution has no value",
            ))?;
        let bytes = canonical_json_bytes(value).map_err(|_| {
            RuntimeChatBridgeError::Admission(
                "turn context contribution could not be canonically encoded",
            )
        })?;
        let instruction = format!(
            "Canonical context contribution from {capability_id}: {}",
            String::from_utf8(bytes).map_err(|_| {
                RuntimeChatBridgeError::Admission(
                    "turn context contribution is not valid UTF-8 JSON",
                )
            })?
        );
        total = total.saturating_add(instruction.len());
        if total > MAX_CONTEXT_INSTRUCTION_BYTES {
            return Err(RuntimeChatBridgeError::Admission(
                "turn context contributions exceed the model context limit",
            ));
        }
        instructions.push(instruction);
    }
    Ok(instructions)
}

fn responses_request(params: &RuntimeStartTurnParams, admitted: &AdmittedTurn) -> ResponsesBridgeRequest {
    ResponsesBridgeRequest {
        bridge_version: nomifun_agent_contracts::VersionString::from(
            CHAT_MODEL_CONTRACT_VERSION,
        ),
        causality: ChatCausality {
            agent_session_id: params.context.agent_session_id.clone(),
            turn_operation_id: admitted.turn_operation_id.clone(),
            causation_event_id: params.input_event_id.clone(),
            resolved_snapshot_ref: params.context.resolved_snapshot_ref.clone(),
            route_identity: admitted.route_identity.clone(),
            operation_id: model_operation_id(&admitted.turn_operation_id),
        },
        model_route_id: admitted.route_identity.route_id.clone(),
        model_route_revision: admitted.route_identity.route_revision,
        instructions: admitted.instructions.clone(),
        input: vec![ResponsesInputItem::Message {
            role: nomifun_chat_model_broker::ResponsesRole::User,
            content: vec![ResponsesInputContent::InputText {
                text: admitted.input_text.clone(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ChatToolChoice::None,
        max_output_tokens: Some(1024),
        reasoning: None,
        prompt_cache: PromptCachePolicy::Disabled,
        response_format: ChatResponseFormat::Text,
        requested_output_modalities: BTreeSet::from([ChatModality::Text]),
        previous_response_id: None,
        preserve_native_responses_items: false,
        metadata: Default::default(),
        store: false,
    }
}

/// Runtime-port decorator used by an app host that owns the production
/// broker.  It leaves Runtime lifecycle/command transport untouched and starts
/// the host bridge only after the sidecar accepts `StartTurn`.
type RuntimeBridgeTaskKey = (RuntimeBindingId, OperationId);

#[derive(Default)]
struct RuntimeBridgeTaskRegistry {
    tasks: BTreeMap<RuntimeBridgeTaskKey, tokio::task::AbortHandle>,
    cancelled: BTreeSet<RuntimeBridgeTaskKey>,
    closed_bindings: BTreeSet<RuntimeBindingId>,
    closed: bool,
}

impl RuntimeBridgeTaskRegistry {
    fn start_if_open(
        &mut self,
        key: RuntimeBridgeTaskKey,
        start: impl FnOnce() -> tokio::task::AbortHandle,
    ) -> bool {
        if self.closed
            || self.closed_bindings.contains(&key.0)
            || self.cancelled.contains(&key)
            || self.tasks.contains_key(&key)
        {
            return false;
        }
        self.tasks.insert(key, start());
        true
    }

    fn finish(&mut self, key: &RuntimeBridgeTaskKey) {
        self.tasks.remove(key);
    }

    fn cancel(
        &mut self,
        key: RuntimeBridgeTaskKey,
    ) -> Option<tokio::task::AbortHandle> {
        self.cancelled.insert(key.clone());
        self.tasks.remove(&key)
    }

    fn close_binding(
        &mut self,
        runtime_binding_id: &RuntimeBindingId,
    ) -> Vec<tokio::task::AbortHandle> {
        self.closed_bindings.insert(runtime_binding_id.clone());
        let keys = self
            .tasks
            .keys()
            .filter(|(binding, _)| binding == runtime_binding_id)
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| self.tasks.remove(&key))
            .collect()
    }

    fn close(&mut self) -> Vec<tokio::task::AbortHandle> {
        self.closed = true;
        std::mem::take(&mut self.tasks)
            .into_values()
            .collect()
    }
}

pub struct BrokerBackedRuntimePort {
    delegate: Arc<dyn CodexRuntimePort>,
    bridge: Arc<RuntimeStartTurnBrokerBridge>,
    task_registry: Arc<Mutex<RuntimeBridgeTaskRegistry>>,
}

impl BrokerBackedRuntimePort {
    pub fn new(
        delegate: Arc<dyn CodexRuntimePort>,
        bridge: Arc<RuntimeStartTurnBrokerBridge>,
    ) -> Self {
        Self {
            delegate,
            bridge,
            task_registry: Arc::new(Mutex::new(RuntimeBridgeTaskRegistry::default())),
        }
    }

    async fn start_bridge_task(
        &self,
        runtime_binding_id: RuntimeBindingId,
        params: RuntimeStartTurnParams,
    ) {
        let key = (
            runtime_binding_id,
            params.context.operation_id.clone(),
        );
        let mut registry = self.task_registry.lock().await;
        let bridge = Arc::clone(&self.bridge);
        let registry_for_completion = Arc::clone(&self.task_registry);
        let task_key = key.clone();
        registry.start_if_open(key, move || {
            let task = tokio::spawn(async move {
                if let Err(error) = bridge.run(params).await {
                    tracing::error!(?error, "Runtime StartTurn broker bridge failed");
                }
                registry_for_completion.lock().await.finish(&task_key);
            });
            task.abort_handle()
        });
    }

    async fn abort_task(
        &self,
        runtime_binding_id: &RuntimeBindingId,
        operation_id: &OperationId,
    ) {
        let key = (runtime_binding_id.clone(), operation_id.clone());
        if let Some(handle) = self.task_registry.lock().await.cancel(key) {
            handle.abort();
        }
    }

    async fn abort_binding_tasks(&self, runtime_binding_id: &RuntimeBindingId) {
        let handles = self
            .task_registry
            .lock()
            .await
            .close_binding(runtime_binding_id);
        for handle in handles {
            handle.abort();
        }
    }

    async fn abort_all_tasks(&self) {
        let handles = self.task_registry.lock().await.close();
        for handle in handles {
            handle.abort();
        }
    }
}

#[async_trait]
impl CodexRuntimePort for BrokerBackedRuntimePort {
    async fn launch(
        &self,
        request: RuntimeLaunchRequest,
    ) -> Result<Arc<ManagedRuntimeSession>, RuntimeError> {
        self.delegate.launch(request).await
    }

    async fn binding(
        &self,
        runtime_binding_id: &RuntimeBindingId,
    ) -> Option<RuntimeBindingContract> {
        self.delegate.binding(runtime_binding_id).await
    }

    async fn command(
        &self,
        runtime_binding_id: &RuntimeBindingId,
        command: &RuntimeCommand,
    ) -> Result<Value, RuntimeError> {
        let start_turn = match command {
            RuntimeCommand::StartTurn(params) => Some(params.clone()),
            _ => None,
        };
        match command {
            RuntimeCommand::Cancel(params) => {
                self.abort_task(runtime_binding_id, &params.target_operation_id)
                    .await;
            }
            RuntimeCommand::SessionDispose(_) => {
                self.abort_binding_tasks(runtime_binding_id).await;
            }
            _ => {}
        }
        let result = self.delegate.command(runtime_binding_id, command).await?;
        match command {
            RuntimeCommand::StartTurn(_) => {
                if let Some(params) = start_turn {
                    self.start_bridge_task(runtime_binding_id.clone(), params)
                        .await;
                }
            }
            _ => {}
        }
        Ok(result)
    }

    async fn dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError> {
        let runtime_binding_id = params.runtime_binding_id.clone();
        self.abort_binding_tasks(&runtime_binding_id).await;
        let result = self.delegate.dispose(params).await;
        result
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.abort_all_tasks().await;
        self.delegate.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::stream;
    use nomifun_agent_contracts::{
        AgentBindingValue, AgentPresetId, AgentSessionLiveRecord, AgentSessionMetadata,
        DigestHex, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        RuntimeCommandContext, RuntimeEventEnvelope, RuntimeProfileKind, VersionString,
    };
    use nomifun_agent_session::{CreateSessionRequest, RuntimeAppendContext};
    use nomifun_chat_model_broker::{
        AnthropicAdapter, BedrockAdapter, BrokerRetryPolicy, ChatCausalityGate,
        ChatModelBroker, ChatModelError, ChatModelErrorCode, ChatProtocol, ChatProtocolAdapter,
        ChatRetryDirective, ChatRouteResolver, ChatRouteSelection, CredentialLease,
        CredentialTarget, GeminiAdapter, OpenAiChatAdapter, OpenAiResponsesAdapter,
        ProviderCredentialRef, ProviderCredentialStore, ProviderIdRef, ProviderTransport,
        ProviderWireFrame, ProviderWireRequest, ProviderWireStream, ResolvedChatRoute,
        ResolvedChatRouteSet, VertexAdapter, protocol_features,
    };
    use serde_json::json;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    use super::*;
    use crate::{ChatOperationClaimStore, ProductionChatCausalityGate};

    fn registry_key() -> RuntimeBridgeTaskKey {
        (
            RuntimeBindingId::from("registry-binding"),
            OperationId::from("registry-operation"),
        )
    }

    #[tokio::test]
    async fn runtime_bridge_registry_cancel_before_insert_is_fenced() {
        let mut registry = RuntimeBridgeTaskRegistry::default();
        let key = registry_key();

        assert!(registry.cancel(key.clone()).is_none());
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_for_task = Arc::clone(&started);
        assert!(!registry.start_if_open(key, move || {
            started_for_task.store(true, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async {}).abort_handle()
        }));

        assert!(!started.load(std::sync::atomic::Ordering::SeqCst));
        assert!(registry.tasks.is_empty());
    }

    #[tokio::test]
    async fn runtime_bridge_registry_cancel_after_insert_aborts_and_removes_task() {
        let mut registry = RuntimeBridgeTaskRegistry::default();
        let key = registry_key();

        assert!(registry.start_if_open(key.clone(), || {
            tokio::spawn(std::future::pending::<()>()).abort_handle()
        }));
        assert_eq!(registry.tasks.len(), 1);

        let handle = registry.cancel(key);
        assert!(handle.is_some());
        handle.expect("task handle was present").abort();
        assert!(registry.tasks.is_empty());
    }

    #[tokio::test]
    async fn runtime_bridge_registry_shutdown_before_insert_is_fenced() {
        let mut registry = RuntimeBridgeTaskRegistry::default();
        let key = registry_key();

        assert!(registry.close().is_empty());
        assert!(!registry.start_if_open(key, || {
            tokio::spawn(async {}).abort_handle()
        }));
        assert!(registry.tasks.is_empty());
    }

    #[tokio::test]
    async fn runtime_bridge_registry_binding_close_before_insert_is_fenced() {
        let mut registry = RuntimeBridgeTaskRegistry::default();
        let key = registry_key();

        assert!(registry.close_binding(&key.0).is_empty());
        assert!(!registry.start_if_open(key, || {
            tokio::spawn(async {}).abort_handle()
        }));
        assert!(registry.tasks.is_empty());
    }

    struct StaticRouteResolver {
        route: ResolvedChatRoute,
    }

    #[async_trait]
    impl ChatRouteResolver for StaticRouteResolver {
        async fn resolve(
            &self,
            _selection: &ChatRouteSelection,
        ) -> Result<ResolvedChatRouteSet, ChatModelError> {
            Ok(ResolvedChatRouteSet {
                primary: self.route.clone(),
                failovers: Vec::new(),
            })
        }
    }

    struct StaticCredentialStore;

    #[async_trait]
    impl ProviderCredentialStore for StaticCredentialStore {
        async fn lease(
            &self,
            credential_ref: &ProviderCredentialRef,
            target: &CredentialTarget,
        ) -> Result<CredentialLease, ChatModelError> {
            Ok(CredentialLease::new(
                credential_ref.clone(),
                target.clone(),
                "runtime-bridge-test-credential",
            ))
        }
    }

    enum TransportOutcome {
        Success,
        Failure,
    }

    struct StaticTransport {
        outcome: TransportOutcome,
    }

    #[async_trait]
    impl ProviderTransport for StaticTransport {
        async fn open_stream(
            &self,
            _request: ProviderWireRequest,
            _credential: CredentialLease,
        ) -> Result<ProviderWireStream, ChatModelError> {
            match self.outcome {
                TransportOutcome::Success => Ok(Box::pin(stream::iter([
                    Ok(ProviderWireFrame {
                        event: "response.start".to_owned(),
                        data: json!({"id": "runtime-bridge-response"}),
                    }),
                    Ok(ProviderWireFrame {
                        event: "output_text.delta".to_owned(),
                        data: json!({"text": "bridge success"}),
                    }),
                    Ok(ProviderWireFrame {
                        event: "usage".to_owned(),
                        data: json!({"input_tokens": 1, "output_tokens": 2}),
                    }),
                    Ok(ProviderWireFrame {
                        event: "response.completed".to_owned(),
                        data: json!({"finish_reason": "stop"}),
                    }),
                ]))),
                TransportOutcome::Failure => Err(ChatModelError::new(
                    ChatModelErrorCode::ProviderUnavailable,
                    "synthetic provider failure",
                    ChatRetryDirective::Never,
                )),
            }
        }
    }

    struct StoreClaim {
        sessions: Arc<AgentSessionStore>,
    }

    #[async_trait]
    impl ChatOperationClaimStore for StoreClaim {
        async fn claim(
            &self,
            request: nomifun_agent_session::ChatOperationClaimRequest,
        ) -> Result<(), ChatModelError> {
            match self.sessions.claim_chat_operation(request).await {
                Ok(result) if result.duplicate => Err(ChatModelError::new(
                    ChatModelErrorCode::DuplicateOperation,
                    "duplicate model operation",
                    ChatRetryDirective::Never,
                )),
                Ok(_) => Ok(()),
                Err(error) => Err(ChatModelError::new(
                    ChatModelErrorCode::CausalityRejected,
                    error.to_string(),
                    ChatRetryDirective::Never,
                )),
            }
        }
    }

    fn route() -> ResolvedChatRoute {
        ResolvedChatRoute {
            model_route_id: "runtime-bridge-route".into(),
            model_route_revision: 1,
            provider_id: ProviderIdRef::from("runtime-bridge-provider"),
            model: "runtime-bridge-model".to_owned(),
            protocol: ChatProtocol::OpenaiChat,
            connection_config_ref: "runtime-bridge-connection".into(),
            config_revision_digest: "a".repeat(64).into(),
            credential_ref: ProviderCredentialRef::from("runtime-bridge-credential"),
            features: protocol_features(ChatProtocol::OpenaiChat),
        }
    }

    fn broker(
        sessions: Arc<AgentSessionStore>,
        outcome: TransportOutcome,
    ) -> Arc<dyn ChatBrokerPort> {
        let transport: Arc<dyn ProviderTransport> = Arc::new(StaticTransport { outcome });
        let adapters: Vec<Arc<dyn ChatProtocolAdapter>> = vec![
            Arc::new(AnthropicAdapter::new(transport.clone())),
            Arc::new(OpenAiChatAdapter::new(transport.clone())),
            Arc::new(OpenAiResponsesAdapter::new(transport.clone())),
            Arc::new(GeminiAdapter::new(transport.clone())),
            Arc::new(BedrockAdapter::new(transport.clone())),
            Arc::new(VertexAdapter::new(transport)),
        ];
        let gate: Arc<dyn ChatCausalityGate> = Arc::new(
            ProductionChatCausalityGate::primary(
                sessions.clone(),
                Arc::new(StoreClaim { sessions }),
            ),
        );
        Arc::new(
            ChatModelBroker::new(
                gate,
                Arc::new(StaticRouteResolver { route: route() }),
                Arc::new(StaticCredentialStore),
                adapters,
                BrokerRetryPolicy {
                    max_total_attempts: 1,
                    max_attempts_per_route: 1,
                },
            )
            .unwrap(),
        )
    }

    async fn admitted_turn(
    ) -> (
        Arc<AgentSessionStore>,
        RuntimeStartTurnParams,
    ) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(nomifun_agent_contracts::FRESH_V4_BASELINE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO schema_metadata (
                singleton_key, data_generation, root_instance_id, migration_head,
                seed_manifest_digest, canonical_schema_manifest_digest,
                projection_schema_version
             ) VALUES ('canonical', 4, 'runtime-bridge-test-root', 1, ?, ?, 1)",
        )
        .bind("0".repeat(64))
        .bind("1".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
        let sessions = Arc::new(AgentSessionStore::from_pool(pool).await.unwrap());
        let session_id = AgentSessionId::from(Uuid::now_v7().to_string());
        let snapshot = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("runtime-bridge-snapshot"),
            snapshot_digest: DigestHex::from("b".repeat(64)),
        };
        let route_identity = ChatRouteIdentity::new(
            "runtime-bridge-preset@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            route().model_route_id,
            1,
        );
        let binding = AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from("runtime-bridge-preset"),
                revision: 1,
                revision_digest: DigestHex::from("c".repeat(64)),
            },
            resolved_snapshot_ref: snapshot.clone(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        let create = sessions
            .create_session(CreateSessionRequest {
                session: AgentSessionLiveRecord {
                    agent_session_id: session_id.clone(),
                    owner_ref: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: "runtime-bridge-owner".to_owned(),
                    },
                    metadata: AgentSessionMetadata {
                        title: None,
                        archived: false,
                        pinned: false,
                    },
                    agent_binding: binding,
                    remote_binding_provenance: None,
                    parent_session_id: None,
                    fork_base_payload_id: None,
                    next_seq: 1,
                },
                created_at: 0,
                operation_id: OperationId::from("runtime-bridge-open"),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key: IdempotencyKey::from("runtime-bridge-open"),
                correlation_id: CorrelationId::from("runtime-bridge-open"),
                initial_input: None,
                opening_event_id: Some(EventId::from("runtime-bridge-opening")),
                activation_event_id: Some(EventId::from("runtime-bridge-active-set")),
                initial_active_capability_ids: Vec::new(),
            })
            .await
            .unwrap();
        let runtime_binding_id = RuntimeBindingId::from("runtime-bridge-binding");
        let runtime_profile_digest = DigestHex::from("d".repeat(64));
        let runtime_bound = EventId::from("runtime-bridge-bound");
        sessions
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id: session_id.clone(),
                envelope: RuntimeEventEnvelope {
                    runtime_binding_id: runtime_binding_id.clone(),
                    producer_seq: 1,
                    event_id: runtime_bound.clone(),
                    idempotency_key: IdempotencyKey::from("runtime-bridge-bound"),
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind("runtime/bound".to_owned()),
                        kind_version: 1,
                        correlation_id: CorrelationId::from(
                            runtime_binding_id.as_ref().to_owned(),
                        ),
                        causation_event_id: Some(create.opening_ack.event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                            "runtime_binding_id": runtime_binding_id,
                            "agent_session_id": session_id,
                            "resolved_snapshot_ref": snapshot,
                            "runtime_release_digest": "e".repeat(64),
                            "runtime_build_digest": "f".repeat(64),
                            "protocol_version": VersionString::from("1.0.0"),
                            "profile_kind": RuntimeProfileKind::ManagedMinimal,
                            "runtime_profile_digest": runtime_profile_digest,
                            "active_set_generation": 0,
                            "runtime_bound_event_id": runtime_bound,
                            "through_seq": 0
                        }))),
                    },
                },
            })
            .await
            .unwrap();
        sessions
            .append_event(&SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: EventId::from("runtime-bridge-ready"),
                producer_id: EventProducerId::from(RUNTIME_SUPERVISOR),
                idempotency_key: IdempotencyKey::from("runtime-bridge-ready"),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("session/ready".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                    causation_event_id: Some(runtime_bound),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
                },
            })
            .await
            .unwrap();

        let turn_operation_id = OperationId::from("runtime-bridge-turn");
        let input_event_id = EventId::from("runtime-bridge-input");
        let idempotency_key = IdempotencyKey::from("runtime-bridge-input");
        sessions
            .append_event(&SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: input_event_id.clone(),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key: idempotency_key.clone(),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("message/user-accepted".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(turn_operation_id.as_ref().to_owned()),
                    causation_event_id: Some(EventId::from("runtime-bridge-ready")),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "content": "hello bridge"
                    }))),
                },
            })
            .await
            .unwrap();
        sessions
            .append_event(&SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: EventId::from("runtime-bridge-turn-started"),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key: IdempotencyKey::from("runtime-bridge-turn-started"),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("turn/started".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(turn_operation_id.as_ref().to_owned()),
                    causation_event_id: Some(input_event_id.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id": turn_operation_id,
                        "input_event_id": input_event_id,
                        "route_identity": route_identity,
                        "resolved_snapshot_ref": snapshot
                    }))),
                },
            })
            .await
            .unwrap();

        (
            sessions,
            RuntimeStartTurnParams {
                context: RuntimeCommandContext {
                    agent_session_id: session_id,
                    runtime_binding_id,
                    operation_id: turn_operation_id,
                    resolved_snapshot_ref: snapshot,
                    runtime_profile_digest,
                    active_set_generation: 0,
                },
                idempotency_key,
                input_event_id,
            },
        )
    }

    #[tokio::test]
    async fn broker_bridge_projects_success_to_one_durable_terminal() {
        let (sessions, params) = admitted_turn().await;
        let bridge = RuntimeStartTurnBrokerBridge::new(
            sessions.clone(),
            broker(sessions.clone(), TransportOutcome::Success),
        );
        let result = bridge.run(params.clone()).await.unwrap();
        assert_eq!(result.content, "bridge success");
        assert_eq!(result.part_count, 1);
        assert!(matches!(
            result.terminal,
            RuntimeTurnTerminal::Completed { .. }
        ));

        let observation = sessions
            .observe(&params.context.agent_session_id, None, 500)
            .await
            .unwrap();
        assert_eq!(observation.head.status, "ready");
        assert_eq!(
            observation
                .events
                .iter()
                .filter(|event| event.kind.0 == "turn/completed")
                .count(),
            1
        );
        assert!(observation.events.iter().any(|event| {
            event.kind.0 == "message/content-part"
        }));
    }

    #[tokio::test]
    async fn broker_bridge_projects_provider_failure_without_fake_output() {
        let (sessions, params) = admitted_turn().await;
        let bridge = RuntimeStartTurnBrokerBridge::new(
            sessions.clone(),
            broker(sessions.clone(), TransportOutcome::Failure),
        );
        let result = bridge.run(params.clone()).await.unwrap();
        assert_eq!(result.content, "");
        assert_eq!(result.part_count, 0);
        assert!(matches!(
            result.terminal,
            RuntimeTurnTerminal::Failed {
                code: ChatModelErrorCode::ProviderUnavailable,
                ..
            }
        ));

        let observation = sessions
            .observe(&params.context.agent_session_id, None, 500)
            .await
            .unwrap();
        assert_eq!(observation.head.status, "ready");
        assert_eq!(
            observation
                .events
                .iter()
                .filter(|event| event.kind.0 == "turn/failed")
                .count(),
            1
        );
        assert!(observation
            .events
            .iter()
            .all(|event| event.kind.0 != "message/content-part"));
    }
}
