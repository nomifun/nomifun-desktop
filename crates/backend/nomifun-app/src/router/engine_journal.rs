//! Engine-neutral journal backed exclusively by the canonical Agent Store.
//!
//! Runtime-private records are durable `runtime/progress-recorded` facts.
//! User-visible assistant text, thinking, and Turn terminals are projected from the same
//! ordered event stream; no Conversation delivery/runtime table participates.

use std::{
    collections::BTreeMap,
    sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, ArtifactId, ChatRouteIdentity, CorrelationId, EventId,
    EventProducerId, IdempotencyKey, OperationId, ResolvedSnapshotRef,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef, SessionPayloadBody, SessionPayloadRecord,
    StrictJsonValue, canonical_json_bytes, digest_bytes,
};
use nomifun_agent_session::{
    AgentSessionStore, ChatOperationClaimRequest, TurnReceiptStatus,
};
use nomifun_chat_model_broker::{
    ChatCausality, ChatCausalityGate, ChatModelError, ChatModelErrorCode,
    ChatRetryDirective,
};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_common::{AppError, now_ms};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::engine_session_host::EngineTurnReceipt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineJournalWrite {
    Progress,
    Settlement,
    Cleanup,
    Terminal,
}

#[derive(Default)]
struct Cursor {
    sequence: u64,
    bytes: usize,
    draining: bool,
    terminal: bool,
    uncertain: bool,
    assistant_step: Option<AssistantStepCursor>,
    last_assistant_event_id: Option<EventId>,
    assistant_message_id: Option<String>,
    thinking_messages: BTreeMap<u16, (String, u64)>,
    tool_message_ids: BTreeMap<String, String>,
}

struct AssistantStepCursor {
    step: u16,
    message_id: String,
    parts: u64,
    text: Vec<u8>,
    last_event_id: Option<EventId>,
}

pub(super) struct Journal {
    store: AgentSessionStore,
    user: String,
    session: AgentSessionId,
    operation: OperationId,
    /// Accepted user message used by model causality and runtime records.
    root: EventId,
    /// Exact canonical turn/started predecessor for Action/Effect chains.
    turn_started: EventId,
    generation: i64,
    snapshot: ResolvedSnapshotRef,
    route: Option<ChatRouteIdentity>,
    cancellation: CancellationToken,
    cursor: Mutex<Cursor>,
    sequence: AtomicU64,
    pending: Arc<Semaphore>,
    pending_bytes: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct EngineTurnJournal(Arc<Journal>);

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine journal: {message}"))
}

/// Derive the one assistant message identity owned by a canonical Turn.  The
/// HTTP/WS relay and durable journal must agree before either side publishes a
/// frame; otherwise terminal history refresh leaves a duplicate live row.
pub(super) fn canonical_assistant_message_id(
    root_message_id: &str,
) -> Result<String, AppError> {
    canonical_assistant_step_message_id(root_message_id, 1)
}

pub(super) fn canonical_assistant_step_message_id(
    root_message_id: &str,
    step: u16,
) -> Result<String, AppError> {
    let root = Uuid::parse_str(root_message_id)
        .map_err(|error| failure(format!("turn root is not a UUID: {error}")))?;
    if root.get_version_num() != 7 || step == 0 {
        return Err(failure("turn root is not UUIDv7"));
    }
    let mut bytes = *root.as_bytes();
    // Preserve version/variant/time ordering while selecting a deterministic,
    // distinct point in the UUIDv7 random suffix for each model step. Step 1
    // retains the existing canonical assistant ID used by the live relay.
    let suffix = u16::from_be_bytes([bytes[14], bytes[15]]) ^ step;
    let suffix_bytes = suffix.to_be_bytes();
    bytes[14] = suffix_bytes[0];
    bytes[15] = suffix_bytes[1];
    let assistant = Uuid::from_bytes(bytes);
    if assistant == root || assistant.get_version_num() != 7 {
        return Err(failure("assistant message identity derivation failed"));
    }
    Ok(assistant.to_string())
}

fn canonical_event_payload(
    session_id: &AgentSessionId,
    value: Value,
) -> Result<(SessionEventPayloadRef, Option<SessionPayloadRecord>), AppError> {
    let bytes = canonical_json_bytes(&value).map_err(failure)?;
    if bytes.len() <= nomifun_agent_session::MAX_INLINE_JSON_BYTES {
        return Ok((
            SessionEventPayloadRef::InlineJson(StrictJsonValue(value)),
            None,
        ));
    }
    if bytes.len() > nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES {
        return Err(failure(format!(
            "record exceeds canonical payload limit of {} bytes",
            nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES,
        )));
    }
    let digest = digest_bytes(&bytes);
    // `payload_id` is global while payload ownership is Session-scoped.
    let payload_id = ArtifactId::from(format!(
        "runtime-progress:{}:{}",
        session_id.as_ref(),
        digest.as_ref()
    ));
    let payload = SessionPayloadRecord {
        payload_id: payload_id.clone(),
        agent_session_id: session_id.clone(),
        media_type: "application/json".to_owned(),
        byte_len: bytes.len() as u64,
        digest,
        body: SessionPayloadBody::Json(StrictJsonValue(value)),
    };
    Ok((SessionEventPayloadRef::Stored(payload_id), Some(payload)))
}

impl EngineTurnJournal {
    pub(super) async fn require_claimed_model(
        &self,
        causality: &ChatCausality,
    ) -> Result<(), AppError> {
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain
            || cursor.terminal
            || cursor.draining
            || journal.cancellation.is_cancelled()
            || causality.agent_session_id != journal.session
            || causality.turn_operation_id != journal.operation
            || causality.causation_event_id != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref()
        {
            return Err(failure("resource request differs from the live model turn"));
        }
        drop(cursor);
        let facts = journal
            .store
            .chat_causality_facts(&journal.session, &journal.operation)
            .await
            .map_err(failure)?;
        if facts.head.status != "running"
            || facts.head.active_turn_id.as_deref() != Some(journal.operation.as_ref())
            || !facts.operation_ids.contains(causality.operation_id.as_ref())
            || journal
                .store
                .has_unsettled_effects(&journal.session)
                .await
                .map_err(failure)?
        {
            return Err(failure("resource request has no claimed model operation"));
        }
        Ok(())
    }

    pub(super) fn validate_receipt(&self, receipt: &EngineTurnReceipt) -> Result<(), AppError> {
        if self.0.session.as_ref() != receipt.session().session().conversation_id
            || self.0.operation.as_ref() != receipt.operation_id()
        {
            return Err(failure("journal belongs to another resource turn"));
        }
        Self::from_existing(self.0.clone(), receipt).map(|_| ())
    }

    pub(super) fn matches_tool(
        &self,
        invocation: &nomifun_engine_core::EngineToolInvocation,
    ) -> bool {
        invocation.agent_session_id == self.0.session
            && invocation.principal.principal_kind == "user"
            && invocation.principal.principal_id == self.0.user
            && invocation.resolved_snapshot_ref == self.0.snapshot
            && invocation.turn_operation_id == self.0.operation
    }

    pub(super) fn downgrade(&self) -> std::sync::Weak<Journal> {
        Arc::downgrade(&self.0)
    }

    pub(super) fn from_existing(
        journal: Arc<Journal>,
        receipt: &EngineTurnReceipt,
    ) -> Result<Self, AppError> {
        if journal.user != receipt.session().principal().principal_id
            || journal.generation != receipt.admission_epoch()
            || journal.root.as_ref() != receipt.root_message_id()
            || journal.turn_started.as_ref() != receipt.turn_started_event_id()
            || journal.snapshot != receipt.session().snapshot().snapshot_ref
        {
            return Err(failure("receipt changed for existing journal"));
        }
        Ok(Self(journal))
    }

    pub(super) fn new(
        store: AgentSessionStore,
        receipt: &EngineTurnReceipt,
        cancellation: CancellationToken,
    ) -> Result<Self, AppError> {
        let assistant_message_id = canonical_assistant_message_id(receipt.root_message_id())?;
        Ok(Self(Arc::new(Journal {
            store,
            user: receipt.session().principal().principal_id.clone(),
            session: AgentSessionId::from(
                receipt.session().session().conversation_id.clone(),
            ),
            operation: OperationId::from(receipt.operation_id().to_owned()),
            root: EventId::from(receipt.root_message_id().to_owned()),
            turn_started: EventId::from(receipt.turn_started_event_id().to_owned()),
            generation: receipt.admission_epoch(),
            snapshot: receipt.session().snapshot().snapshot_ref.clone(),
            route: receipt
                .session()
                .snapshot()
                .content
                .chat_route_identity
                .clone(),
            cancellation,
            cursor: Mutex::new(Cursor {
                assistant_message_id: Some(assistant_message_id),
                ..Cursor::default()
            }),
            sequence: AtomicU64::new(0),
            pending: Arc::new(Semaphore::new(64)),
            pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
        })))
    }

    pub fn sequence(&self) -> u64 {
        self.0.sequence.load(Ordering::Acquire)
    }

    async fn append_progress(
        journal: &Journal,
        cursor: &Cursor,
        event: &Value,
    ) -> Result<(), AppError> {
        let next = cursor.sequence.saturating_add(1);
        let value = json!({
            "runtime_binding_id": format!("nomi:{}", journal.session.as_ref()),
            "producer_seq": next,
            "event": event,
        });
        let (payload_ref, payload) = canonical_event_payload(&journal.session, value)?;
        let identity = format!(
            "runtime-progress:{}:{}:{next}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/progress-recorded".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(journal.operation.as_ref().to_owned()),
                causation_event_id: Some(journal.root.clone()),
                payload: payload_ref,
            },
        };
        journal
            .store
            .append_event_with_payload(&append, payload.as_ref())
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn append_assistant_part(
        journal: &Journal,
        cursor: &mut Cursor,
        step: u16,
        text: &str,
    ) -> Result<(), AppError> {
        if text.is_empty() {
            return Ok(());
        }
        if cursor.assistant_step.as_ref().is_some_and(|current| current.step != step) {
            let previous = cursor.assistant_step.take().expect("checked above");
            if step <= previous.step {
                return Err(failure("assistant model step moved backwards"));
            }
            let completed = Self::assistant_completion_event(journal, &previous);
            let completed_id = completed.event_id.clone();
            journal.store.append_event(&completed).await.map_err(failure)?;
            cursor.last_assistant_event_id = Some(completed_id);
        }
        if cursor.assistant_step.is_none() {
            cursor.assistant_step = Some(AssistantStepCursor {
                step,
                message_id: canonical_assistant_step_message_id(journal.root.as_ref(), step)?,
                parts: 0,
                text: Vec::new(),
                last_event_id: None,
            });
        }
        let current = cursor.assistant_step.as_mut().expect("initialized above");
        let part = current.parts.saturating_add(1);
        let message_id = current.message_id.clone();
        let identity = format!(
            "assistant-part:{}:{}:{step}:{part}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let event_id = EventId::from(identity.clone());
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: event_id.clone(),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/content-part".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id),
                causation_event_id: Some(
                    cursor
                        .last_assistant_event_id
                        .clone()
                        .unwrap_or_else(|| journal.root.clone()),
                ),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "content": text,
                    "turn_id": journal.root.as_ref(),
                    "display_at_ms": now_ms(),
                }))),
            },
        };
        journal.store.append_event(&append).await.map_err(failure)?;
        current.parts = part;
        current.text.extend_from_slice(text.as_bytes());
        current.last_event_id = Some(event_id.clone());
        cursor.last_assistant_event_id = Some(event_id);
        Ok(())
    }

    fn assistant_completion_event(
        journal: &Journal,
        step: &AssistantStepCursor,
    ) -> SessionEventAppend {
        let identity = format!(
            "assistant-complete:{}:{}:{}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
            step.step,
        );
        SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/completed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(step.message_id.clone()),
                causation_event_id: Some(
                    step.last_event_id
                        .clone()
                        .unwrap_or_else(|| journal.root.clone()),
                ),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "part_count": step.parts,
                    "content_digest": digest_bytes(&step.text),
                }))),
            },
        }
    }

    async fn append_thinking_part(
        journal: &Journal,
        cursor: &mut Cursor,
        step: u16,
        text: &str,
    ) -> Result<(), AppError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let (message_id, previous_parts) = cursor
            .thinking_messages
            .entry(step)
            .or_insert_with(|| (Uuid::now_v7().to_string(), 0));
        let part = previous_parts.saturating_add(1);
        let identity = format!(
            "thinking-part:{}:{}:{step}:{part}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let (payload_ref, payload) = canonical_event_payload(
            &journal.session,
            json!({ "content": text, "turn_id": journal.root.as_ref() }),
        )?;
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("thinking/content-part".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id.clone()),
                causation_event_id: Some(journal.turn_started.clone()),
                payload: payload_ref,
            },
        };
        journal
            .store
            .append_event_with_payload(&append, payload.as_ref())
            .await
            .map_err(failure)?;
        *previous_parts = part;
        Ok(())
    }

    async fn append_tool_projection(
        journal: &Journal,
        cursor: &mut Cursor,
        value: &Value,
    ) -> Result<(), AppError> {
        match value.get("event").and_then(Value::as_str) {
            Some("host_tool_dispatch") => {
                let dispatch = value.get("dispatch").and_then(Value::as_object).ok_or_else(|| {
                    failure("host tool dispatch has no canonical payload")
                })?;
                let operation = dispatch
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no operation_id"))?;
                let call_id = dispatch
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no call_id"))?;
                if call_id.starts_with("agent-instructions:") {
                    return Ok(());
                }
                let capability_id = dispatch
                    .get("capability_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no capability_id"))?;
                let action_id = dispatch
                    .get("action_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no action_id"))?;
                let identity = format!(
                    "tool-call:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .entry(operation.to_owned())
                    .or_insert_with(|| Uuid::now_v7().to_string())
                    .clone();
                journal
                    .store
                    .append_event(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/call-started".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(journal.turn_started.clone()),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "capability_id": capability_id,
                                "action_id": action_id,
                                "name": dispatch.get("model_name").cloned().unwrap_or(Value::Null),
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_tool_settled") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool settlement has no operation_id"))?;
                let call_id = value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool settlement has no call_id"))?;
                if call_id.starts_with("agent-instructions:") {
                    return Ok(());
                }
                let identity = format!(
                    "tool-result:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .get(operation)
                    .cloned()
                    .ok_or_else(|| failure("host tool settlement has no admitted projection"))?;
                journal
                    .store
                    .append_event(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/result-recorded".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(EventId::from(format!(
                                "tool-call:{}:{operation}",
                                journal.session.as_ref(),
                            ))),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "output": value.get("result").cloned().unwrap_or(Value::Null),
                                "error": value.get("error").cloned().unwrap_or(Value::Null),
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_resource_dispatch") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource dispatch has no operation_id"))?;
                let call_id = value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource dispatch has no call_id"))?;
                let identity = format!(
                    "tool-call:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .entry(operation.to_owned())
                    .or_insert_with(|| Uuid::now_v7().to_string())
                    .clone();
                journal
                    .store
                    .append_event(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/call-started".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(journal.turn_started.clone()),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "capability_id": "mcp.server",
                                "action_id": "mcp.resource/read",
                                "name": "mcp_resource_read",
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_resource_settled") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource settlement has no operation_id"))?;
                let identity = format!(
                    "tool-result:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .get(operation)
                    .cloned()
                    .ok_or_else(|| failure("host resource settlement has no admitted projection"))?;
                journal
                    .store
                    .append_event(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/result-recorded".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(EventId::from(format!(
                                "tool-call:{}:{operation}",
                                journal.session.as_ref(),
                            ))),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "output": {"owner_returned": value.get("owner_returned").cloned().unwrap_or(Value::Null)},
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn append_terminal(
        journal: &Journal,
        cursor: &Cursor,
        event: &AgentEngineEvent,
    ) -> Result<(), AppError> {
        match event {
            AgentEngineEvent::TurnCancelled { .. } => {
                let receipt = journal
                    .store
                    .read_turn_receipt(&journal.session, &journal.operation)
                    .await
                    .map_err(failure)?;
                if receipt.status != TurnReceiptStatus::Cancelled {
                    journal
                        .store
                        .cancel_active_turn(
                            &journal.session,
                            IdempotencyKey::from(format!(
                                "runtime-cancel:{}:{}",
                                journal.session.as_ref(),
                                journal.operation.as_ref(),
                            )),
                            EventProducerId::from("runtime_supervisor"),
                        )
                        .await
                        .map_err(failure)?;
                }
                Ok(())
            }
            AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnFailed { .. } => {
                let empty_step = AssistantStepCursor {
                    step: 1,
                    message_id: cursor
                        .assistant_message_id
                        .clone()
                        .unwrap_or_else(|| Uuid::now_v7().to_string()),
                    parts: 0,
                    text: Vec::new(),
                    last_event_id: None,
                };
                let message = Self::assistant_completion_event(
                    journal,
                    cursor.assistant_step.as_ref().unwrap_or(&empty_step),
                );
                let message_event_id = message.event_id.clone();
                let (kind, payload) = match event {
                    AgentEngineEvent::TurnCompleted { model_steps, finish_reason } => (
                        "turn/completed",
                        json!({
                            "model_steps": model_steps,
                            "finish_reason": finish_reason,
                            "finished_at_ms": now_ms(),
                        }),
                    ),
                    AgentEngineEvent::TurnFailed { model_steps, message } => {
                        let error = nomifun_ai_agent::AgentSendError::from_engine_turn_failure(
                            message.clone(),
                        )
                        .into_stream_error();
                        (
                            "turn/failed",
                            json!({
                                "model_steps": model_steps,
                                "message": message,
                                "error": error,
                                "finished_at_ms": now_ms(),
                            }),
                        )
                    }
                    _ => unreachable!(),
                };
                let turn_identity = format!(
                    "turn-terminal:{}:{}",
                    journal.session.as_ref(),
                    journal.operation.as_ref(),
                );
                let turn = SessionEventAppend {
                    agent_session_id: journal.session.clone(),
                    event_id: EventId::from(turn_identity.clone()),
                    producer_id: EventProducerId::from("runtime_supervisor"),
                    idempotency_key: IdempotencyKey::from(turn_identity),
                    runtime_binding_id: None,
                    runtime_producer_seq: None,
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind(kind.to_owned()),
                        kind_version: 1,
                        correlation_id: CorrelationId::from(journal.operation.as_ref().to_owned()),
                        causation_event_id: Some(message_event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
                    },
                };
                journal
                    .store
                    .append_chat_completion(&message, &turn, &journal.operation)
                    .await
                    .map_err(failure)?;
                Ok(())
            }
            _ => Err(failure("terminal write did not contain a terminal Runtime event")),
        }
    }

    pub async fn append(
        &self,
        payload: String,
        model_operation: Option<String>,
        kind: EngineJournalWrite,
    ) -> Result<(), AppError> {
        if payload.len() > 8 * 1024 * 1024 {
            return Err(failure("record exceeds hard byte limit"));
        }
        if model_operation
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 1024)
            || (model_operation.is_some() && kind != EngineJournalWrite::Progress)
        {
            return Err(failure("invalid model-operation admission"));
        }
        let event_value: Value = serde_json::from_str(&payload).map_err(failure)?;
        let runtime_event = serde_json::from_value::<AgentEngineEvent>(event_value.clone()).ok();
        if kind == EngineJournalWrite::Terminal && runtime_event.is_none() {
            return Err(failure("terminal write did not contain a terminal Runtime event"));
        }
        let permit = self
            .0
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure("pending write bound reached"))?;
        let byte_permit = self
            .0
            .pending_bytes
            .clone()
            .try_acquire_many_owned(payload.len() as u32)
            .map_err(|_| failure("pending write byte bound reached"))?;
        let journal = self.0.clone();
        let task = tokio::runtime::Handle::try_current().map_err(failure)?.spawn(async move {
            let _permit = permit;
            let _byte_permit = byte_permit;
            let mut cursor = journal.cursor.lock().await;
            if cursor.uncertain || cursor.terminal {
                return Err(failure("journal is closed or uncertain"));
            }
            if kind == EngineJournalWrite::Progress
                && (cursor.draining || journal.cancellation.is_cancelled())
            {
                return Err(failure("turn no longer admits progress"));
            }
            if kind == EngineJournalWrite::Terminal && !cursor.draining {
                return Err(failure("terminal requires host cleanup phase"));
            }
            let reserved = matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal)
                || (kind == EngineJournalWrite::Settlement
                    && (cursor.draining || journal.cancellation.is_cancelled()));
            let (records, bytes) = if reserved {
                (4095_u64, 8 * 1024 * 1024)
            } else {
                (3200_u64, 4 * 1024 * 1024)
            };
            let next_bytes = cursor.bytes.saturating_add(payload.len());
            if cursor.sequence >= records || next_bytes > bytes {
                return Err(failure("bounded evidence journal exhausted"));
            }
            cursor.uncertain = true;
            Self::append_progress(&journal, &cursor, &event_value).await?;
            Self::append_tool_projection(&journal, &mut cursor, &event_value).await?;
            if let Some(AgentEngineEvent::OutputTextDelta { step, text }) = &runtime_event {
                Self::append_assistant_part(&journal, &mut cursor, *step, text).await?;
            }
            if let Some(AgentEngineEvent::ReasoningDelta { step, text }) = &runtime_event {
                Self::append_thinking_part(&journal, &mut cursor, *step, text).await?;
            }
            if kind == EngineJournalWrite::Terminal {
                Self::append_terminal(
                    &journal,
                    &cursor,
                    runtime_event.as_ref().expect("terminal Runtime event checked above"),
                )
                .await?;
            }
            cursor.sequence = cursor.sequence.saturating_add(1);
            cursor.bytes = next_bytes;
            cursor.draining |= matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal);
            cursor.terminal = kind == EngineJournalWrite::Terminal;
            cursor.uncertain = false;
            journal.sequence.store(cursor.sequence, Ordering::Release);
            Ok(())
        });
        task.await.map_err(failure)?
    }
}

#[async_trait]
impl ChatCausalityGate for EngineTurnJournal {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        let reject = |reason: &str| {
            ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                reason,
                ChatRetryDirective::Never,
            )
        };
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain
            || cursor.terminal
            || cursor.draining
            || journal.cancellation.is_cancelled()
            || causality.agent_session_id != journal.session
            || causality.turn_operation_id != journal.operation
            || causality.causation_event_id != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref()
        {
            return Err(reject("model request differs from admitted live turn"));
        }
        drop(cursor);
        if journal
            .store
            .has_unsettled_effects(&journal.session)
            .await
            .map_err(|_| reject("cannot establish durable effect fence"))?
        {
            return Err(reject("an earlier effect has no settled outcome"));
        }
        journal
            .store
            .claim_chat_operation(ChatOperationClaimRequest {
                agent_session_id: journal.session.clone(),
                operation_id: causality.operation_id.clone(),
                turn_operation_id: journal.operation.clone(),
                causation_event_id: journal.root.clone(),
                route_identity: causality.route_identity.clone(),
                resolved_snapshot_ref: causality.resolved_snapshot_ref.clone(),
            })
            .await
            .map_err(|_| reject("model operation already claimed or turn was fenced"))?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) async fn test_fixture() -> (EngineTurnJournal, nomifun_db::SqlitePool) {
    use nomifun_agent_contracts::{
        AgentBindingValue, AgentPresetId, AgentSessionLiveRecord, AgentSessionMetadata,
        DigestHex, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId,
    };
    use nomifun_agent_session::CreateSessionRequest;

    let database = nomifun_db::init_database_memory().await.unwrap();
    let pool = database.pool().clone();
    let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
    let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
    };
    let binding = AgentBindingValue {
        preset_revision_ref: PresetRevisionRef {
            preset_id: AgentPresetId::from("preset"),
            revision: 1,
            revision_digest: DigestHex::from("a".repeat(64)),
        },
        resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("b".repeat(64)),
        },
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let created = store
        .create_session(CreateSessionRequest::new(
            AgentSessionLiveRecord {
                agent_session_id: session_id.clone(),
                owner_ref: owner.clone(),
                metadata: AgentSessionMetadata {
                    title: Some("fixture".into()),
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: binding.clone(),
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            1,
            "open",
            EventProducerId::from("session_api"),
            IdempotencyKey::from("open"),
            CorrelationId::from("open"),
        ))
        .await
        .unwrap();
    let ready = SessionEventAppend {
        agent_session_id: session_id.clone(),
        event_id: EventId::from("ready"),
        producer_id: EventProducerId::from("runtime_supervisor"),
        idempotency_key: IdempotencyKey::from("ready"),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("session/ready".into()),
            kind_version: 1,
            correlation_id: CorrelationId::from("ready"),
            causation_event_id: Some(created.opening_ack.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
        },
    };
    store.append_event(&ready).await.unwrap();
    let input = StrictJsonValue(json!({
        "content": "fixture",
        "admission": {
            "route_identity": {
                "model_task": "chat",
                "provider_id": "provider",
                "model": "model",
                "protocol": "openai_chat_completions",
                "connection_role": "default",
                "capability_revision": 1
            },
            "resolved_snapshot_ref": binding.resolved_snapshot_ref,
        }
    }));
    let (message, turn) = store
        .start_turn(
            &session_id,
            EventProducerId::from("session_api"),
            IdempotencyKey::from("turn-key"),
            OperationId::from("turn"),
            input,
        )
        .await
        .unwrap();
    let root = message.record.unwrap().event_id;
    let turn_started = turn.record.as_ref().unwrap().event_id.clone();
    let journal = EngineTurnJournal(Arc::new(Journal {
        store,
        user: owner.principal_id,
        session: session_id,
        operation: OperationId::from("turn"),
        root,
        turn_started,
        generation: turn.cursor.seq as i64,
        snapshot: binding.resolved_snapshot_ref,
        route: None,
        cancellation: CancellationToken::new(),
        cursor: Mutex::new(Cursor::default()),
        sequence: AtomicU64::new(0),
        pending: Arc::new(Semaphore::new(64)),
        pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
    }));
    (journal, pool)
}

#[cfg(test)]
mod history_display_tests {
    use super::*;
    use super::super::runtime_event_buffer::AgentEventBuffer;

    #[tokio::test]
    async fn public_progress_keeps_model_steps_separate_across_tool_history() {
        let (journal, pool) = test_fixture().await;
        for event in [
            serde_json::to_value(AgentEngineEvent::OutputTextDelta {
                step: 1,
                text: "I found the cause.".into(),
            }).unwrap(),
            json!({
                "event": "host_tool_dispatch",
                "dispatch": {
                    "operation_id": "read-op",
                    "call_id": "read-call",
                    "capability_id": "workspace.files",
                    "action_id": "workspace.files/read",
                    "model_name": "read_file"
                }
            }),
            json!({
                "event": "host_tool_settled",
                "operation_id": "read-op",
                "call_id": "read-call",
                "result": "read complete"
            }),
            serde_json::to_value(AgentEngineEvent::OutputTextDelta {
                step: 2,
                text: "The check passed.".into(),
            }).unwrap(),
        ] {
            journal.append(event.to_string(), None, EngineJournalWrite::Progress)
                .await.unwrap();
        }
        journal.append(json!({"phase":"cleanup"}).to_string(), None, EngineJournalWrite::Cleanup)
            .await.unwrap();
        journal.append(
            serde_json::to_string(&AgentEngineEvent::TurnCompleted {
                model_steps: 2,
                finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed,
            }).unwrap(),
            None,
            EngineJournalWrite::Terminal,
        ).await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store.message_history_before(&journal.0.session, None, 50)
            .await.unwrap();
        let mut notes = history.iter()
            .filter(|row| row.presentation_intent == "message"
                && row.projection["content"].as_str().is_some_and(|text| {
                    text == "I found the cause." || text == "The check passed."
                }))
            .collect::<Vec<_>>();
        notes.sort_by_key(|row| row.first_seq);
        let tool = history.iter().find(|row| row.presentation_intent == "tool")
            .expect("tool history exists");
        assert_eq!(notes.len(), 2);
        assert!(notes[0].first_seq < tool.first_seq);
        assert!(tool.first_seq < notes[1].first_seq);
        assert_ne!(notes[0].projection_id, notes[1].projection_id);
        assert!(notes.iter().all(|row| row.projection["state"] == "completed"));
        assert!(notes.iter().all(|row| row.projection["turn_id"] == journal.0.root.as_ref()));
    }

    #[tokio::test]
    async fn engine_step_limit_projects_a_local_incomplete_turn_error() {
        let (journal, pool) = test_fixture().await;
        journal.append(json!({"phase":"cleanup"}).to_string(), None, EngineJournalWrite::Cleanup)
            .await.unwrap();
        let terminal = AgentEngineEvent::TurnFailed {
            model_steps: 32,
            message: "model step limit of 32 exceeded".into(),
        };
        journal.append(serde_json::to_string(&terminal).unwrap(), None, EngineJournalWrite::Terminal)
            .await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store.message_history_before(&journal.0.session, None, 50)
            .await.unwrap();
        let summary = history.iter().find(|projection| projection.presentation_intent == "turn_summary")
            .expect("terminal summary persisted");
        assert_eq!(summary.projection["error"]["code"], "NOMIFUN_TASK_INCOMPLETE");
        assert_eq!(summary.projection["error"]["ownership"], "nomifun");
    }

    #[tokio::test]
    async fn internal_instruction_preflight_keeps_audit_events_out_of_message_history() {
        let (journal, pool) = test_fixture().await;
        for event in [
            json!({
                "event": "host_tool_dispatch",
                "dispatch": {
                    "operation_id": "instruction-read-1",
                    "call_id": "agent-instructions:100",
                    "capability_id": "workspace.files",
                    "action_id": "workspace.files/read",
                    "model_name": "read_file"
                }
            }),
            json!({
                "event": "host_tool_settled",
                "operation_id": "instruction-read-1",
                "call_id": "agent-instructions:100",
                "error": "instruction discovery failed"
            }),
        ] {
            journal.append(event.to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        }

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        assert!(history.iter().all(|message| message.presentation_intent != "tool"));
        assert_eq!(journal.sequence(), 2);
    }

    #[tokio::test]
    async fn buffered_thinking_survives_a_cold_history_read() {
        let (journal, pool) = test_fixture().await;
        let mut buffer = AgentEventBuffer::default();
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta {
            step: 1,
            text: "Inspect the workspace. ".to_owned(),
        }).is_empty());
        let mut records = Vec::new();
        buffer.flush(&mut records);
        assert_eq!(records.len(), 1);
        journal.append(
            serde_json::to_string(&records[0]).unwrap(),
            None,
            EngineJournalWrite::Progress,
        ).await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        let thinking = history.iter().find(|projection| projection.presentation_intent == "thinking")
            .expect("thinking projection is present in a new history reader");
        assert_eq!(thinking.projection["content"], "Inspect the workspace. ");
        assert_eq!(thinking.projection["turn_id"], journal.0.root.as_ref());
        store.rebuild_projections(&journal.0.session).await.unwrap();
        let (rebuilt, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        assert_eq!(
            rebuilt.iter().find(|projection| projection.presentation_intent == "thinking")
                .unwrap().projection["content"],
            "Inspect the workspace. "
        );
    }
}
