use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata,
    AgentSessionTombstone,
    CanonicalErrorCode, ChatRouteIdentity, CompactionCompletedPayload, CorrelationId, EventId,
    EventProducerId, IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef,
    RuntimeCheckpointValidationResult, RuntimeEventAck, RuntimeEventEnvelope, SessionEventAck,
    SessionEventCursor, SessionEventPayloadRef, SessionEventRecord, SessionForkContract,
    SessionPayloadBody, SessionPayloadId, StrictJsonValue,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub session: AgentSessionLiveRecord,
    pub created_at: i64,
    pub operation_id: OperationId,
    pub producer_id: EventProducerId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_input: Option<StrictJsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_event_id: Option<EventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_event_id: Option<EventId>,
    #[serde(default)]
    pub initial_active_capability_ids: Vec<String>,
}

impl CreateSessionRequest {
    pub fn new(
        session: AgentSessionLiveRecord,
        created_at: i64,
        operation_id: impl Into<OperationId>,
        producer_id: EventProducerId,
        idempotency_key: IdempotencyKey,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            session,
            created_at,
            operation_id: operation_id.into(),
            producer_id,
            idempotency_key,
            correlation_id,
            initial_input: None,
            opening_event_id: None,
            activation_event_id: None,
            initial_active_capability_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCreateResult {
    pub session: AgentSessionLiveRecord,
    pub opening_ack: SessionEventAck,
    pub activation_ack: SessionEventAck,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEventAppendResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<SessionEventRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack: Option<SessionEventAck>,
    pub cursor: SessionEventCursor,
    pub persisted: bool,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEventAppendResult {
    pub append: SessionEventAppendResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack: Option<RuntimeEventAck>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionHeadProjection {
    pub session_id: nomifun_agent_contracts::AgentSessionId,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_turn_id: Option<String>,
    pub active_set_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_checkpoint_locator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_checkpoint_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_bound_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_protocol_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_through_seq: Option<u64>,
    pub last_seq: u64,
    pub unread_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageProjection {
    pub session_id: nomifun_agent_contracts::AgentSessionId,
    pub projection_id: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub presentation_intent: String,
    pub projection: serde_json::Value,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionObservation {
    pub session: AgentSessionLiveRecord,
    pub head: SessionHeadProjection,
    pub events: Vec<SessionEventRecord>,
    pub messages: Vec<MessageProjection>,
    pub next_cursor: SessionEventCursor,
}

/// One atomic, read-only snapshot of the AgentSession facts needed by a
/// provider/chat admission gate.
///
/// The store computes the metadata sets in the same transaction as the live
/// session and head reads.  Callers must not reconstruct these facts from
/// separately-timed `get_live_session`, `head`, and `read_events` calls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCausalityFacts {
    pub session: AgentSessionLiveRecord,
    pub head: SessionHeadProjection,
    pub events: Vec<SessionEventRecord>,
    pub event_payloads: BTreeMap<String, serde_json::Value>,
    pub operation_ids: BTreeSet<String>,
    pub turn_route_identities: BTreeSet<ChatRouteIdentity>,
}

/// Atomic model-operation admission input. The store validates the active
/// turn, causation link, frozen Snapshot, route revision, and operation
/// uniqueness in the same transaction that appends the admission event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatOperationClaimRequest {
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub turn_operation_id: OperationId,
    pub causation_event_id: EventId,
    pub route_identity: ChatRouteIdentity,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEventPage {
    pub agent_session_id: nomifun_agent_contracts::AgentSessionId,
    pub events: Vec<SessionEventRecord>,
    pub next_cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEventRequest {
    pub agent_session_id: nomifun_agent_contracts::AgentSessionId,
    pub event_id: EventId,
    pub producer_id: EventProducerId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_event_id: Option<EventId>,
    pub payload: SessionEventPayloadRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectTerminalState {
    Succeeded,
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectReconcileOutcome {
    ConfirmedSucceeded { receipt: serde_json::Value },
    ConfirmedFailed { error: CanonicalErrorCode },
    StillUncertain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointAdmission {
    pub validation: RuntimeCheckpointValidationResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<nomifun_agent_contracts::SnapshotCompatibilityAdmissionResult>,
    pub checkpoint_reusable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRehydrationInput {
    pub agent_session_id: nomifun_agent_contracts::AgentSessionId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_compaction: Option<CompactionCompletedPayload>,
    pub subsequent_events: Vec<SessionEventRecord>,
    pub through_cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkRequest {
    pub child_session_id: nomifun_agent_contracts::AgentSessionId,
    pub child_owner_ref: PrincipalRef,
    pub child_metadata: AgentSessionMetadata,
    pub child_agent_binding: AgentBindingValue,
    pub parent_through_seq: u64,
    pub created_at: i64,
    pub producer_id: EventProducerId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub event_id: Option<EventId>,
    pub base_payload_id: SessionPayloadId,
    pub base_body: SessionPayloadBody,
    pub base_media_type: String,
    #[serde(default)]
    pub child_initial_active_capability_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkResult {
    pub child_session: AgentSessionLiveRecord,
    pub contract: SessionForkContract,
    pub fork_ack: SessionEventAck,
    pub child_cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteResult {
    pub tombstone: AgentSessionTombstone,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAppendContext {
    pub agent_session_id: nomifun_agent_contracts::AgentSessionId,
    pub envelope: RuntimeEventEnvelope,
}
