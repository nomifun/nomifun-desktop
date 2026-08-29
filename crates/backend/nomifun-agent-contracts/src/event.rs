use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::primitives::{
    AgentSessionId, CanonicalErrorCode, CanonicalSchemaRef, CorrelationId, EventId,
    EventProducerId, IdempotencyKey, ProjectionReducerId, RuntimeBindingId, StrictJsonValue,
    VersionString,
};
use crate::session::SessionPayloadId;

const ACTIVE_SET_COMMITTED: &str = "capability/active-set-committed";
const ACTIVATION_REQUESTED: &str = "capability/activation-requested";
const ACTIVATION_FAILED: &str = "capability/activation-failed";

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SessionEventKind(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventPersistence {
    Persistent,
    TransientDiagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventProducer {
    SessionApi,
    RuntimeSupervisor,
    RuntimeSidecar,
    CapabilityHost,
    OwningPlugin,
    CompactionCoordinator,
    ForkCoordinator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventProducerRule {
    pub allowed: Vec<SessionEventProducer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventPredecessorMode {
    None,
    AnyCommitted,
    AnyOf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventPredecessorRule {
    pub mode: SessionEventPredecessorMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<SessionEventKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventCorrelationRule {
    Session,
    Turn,
    Message,
    ToolCall,
    Effect,
    RuntimeBinding,
    Compaction,
    Fork,
    Optional,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventIdempotencyRule {
    ProducerScopedRequired,
    RuntimeBindingSequenceRequired,
    EffectScopedRequired,
    OperationScopedRequired,
    DiagnosticBestEffort,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventProjectorRule {
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reducers: Vec<ProjectionReducerId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventReplayRule {
    RebuildProjection,
    RehydrateRuntimeInput,
    NoEffect,
    CacheMetadataOnly,
    IgnoreTransientDiagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventRegistryEntry {
    pub kind: SessionEventKind,
    pub version: u32,
    pub persistence: SessionEventPersistence,
    pub payload_schema: CanonicalSchemaRef,
    pub producer: SessionEventProducerRule,
    pub predecessor: SessionEventPredecessorRule,
    pub correlation: SessionEventCorrelationRule,
    pub idempotency: SessionEventIdempotencyRule,
    pub projector: SessionEventProjectorRule,
    pub replay_rule: SessionEventReplayRule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventRegistryPayload {
    pub registry_version: VersionString,
    pub entries: Vec<SessionEventRegistryEntry>,
}

impl SessionEventRegistryPayload {
    pub fn validate(&self) -> Result<(), SessionEventRegistryValidationError> {
        let mut keys = BTreeSet::new();
        let mut persistent_activation_commits = 0_u8;

        for entry in &self.entries {
            let key = (entry.kind.0.clone(), entry.version);
            if !keys.insert(key) {
                return Err(SessionEventRegistryValidationError::DuplicateEntry {
                    kind: entry.kind.0.clone(),
                    version: entry.version,
                });
            }

            if entry.kind.0 == ACTIVE_SET_COMMITTED {
                if entry.persistence != SessionEventPersistence::Persistent {
                    return Err(
                        SessionEventRegistryValidationError::ActivationCommitMustBePersistent,
                    );
                }
                persistent_activation_commits += 1;
            }

            if entry.persistence == SessionEventPersistence::Persistent
                && (entry.kind.0.starts_with("capability/activation-")
                    || entry.kind.0.starts_with("activation/"))
            {
                return Err(
                    SessionEventRegistryValidationError::UnexpectedPersistentActivationEvent {
                        kind: entry.kind.0.clone(),
                    },
                );
            }

            if (entry.kind.0 == ACTIVATION_REQUESTED || entry.kind.0 == ACTIVATION_FAILED)
                && entry.persistence != SessionEventPersistence::TransientDiagnostic
            {
                return Err(
                    SessionEventRegistryValidationError::ActivationDiagnosticMustBeTransient {
                        kind: entry.kind.0.clone(),
                    },
                );
            }
        }

        if persistent_activation_commits != 1 {
            return Err(
                SessionEventRegistryValidationError::PersistentActivationCommitCount {
                    actual: persistent_activation_commits,
                },
            );
        }

        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionEventRegistryValidationError {
    #[error("duplicate SessionEvent registry entry {kind} v{version}")]
    DuplicateEntry { kind: String, version: u32 },
    #[error("capability/active-set-committed must be persistent")]
    ActivationCommitMustBePersistent,
    #[error("{kind} must be transient diagnostic")]
    ActivationDiagnosticMustBeTransient { kind: String },
    #[error("only capability/active-set-committed may persist activation state, found {kind}")]
    UnexpectedPersistentActivationEvent { kind: String },
    #[error(
        "registry must contain exactly one persistent capability/active-set-committed entry, found {actual}"
    )]
    PersistentActivationCommitCount { actual: u8 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "storage", content = "value", rename_all = "snake_case")]
pub enum SessionEventPayloadRef {
    Empty,
    InlineJson(StrictJsonValue),
    Stored(SessionPayloadId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSessionEventDraft {
    pub kind: SessionEventKind,
    pub kind_version: u32,
    pub correlation_id: CorrelationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_event_id: Option<EventId>,
    pub payload: SessionEventPayloadRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventAppend {
    pub agent_session_id: AgentSessionId,
    pub event_id: EventId,
    pub producer_id: EventProducerId,
    pub idempotency_key: IdempotencyKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_binding_id: Option<RuntimeBindingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_producer_seq: Option<u64>,
    pub semantic_event: SemanticSessionEventDraft,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventRecord {
    pub agent_session_id: AgentSessionId,
    pub seq: u64,
    pub event_id: EventId,
    pub producer_id: EventProducerId,
    pub idempotency_key: IdempotencyKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_binding_id: Option<RuntimeBindingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_producer_seq: Option<u64>,
    pub kind: SessionEventKind,
    pub kind_version: u32,
    pub correlation_id: CorrelationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_event_id: Option<EventId>,
    pub payload: SessionEventPayloadRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventCursor {
    pub agent_session_id: AgentSessionId,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEventAck {
    pub agent_session_id: AgentSessionId,
    pub event_id: EventId,
    pub seq: u64,
    pub cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEventEnvelope {
    pub runtime_binding_id: RuntimeBindingId,
    pub producer_seq: u64,
    pub event_id: EventId,
    pub idempotency_key: IdempotencyKey,
    pub semantic_event: SemanticSessionEventDraft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEventAck {
    pub runtime_binding_id: RuntimeBindingId,
    pub committed_producer_seq: u64,
    pub session_event_ack: SessionEventAck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalErrorClass {
    Session,
    Capability,
    Resource,
    Remote,
    Runtime,
    Provider,
    Budget,
    Contract,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorRetryDirective {
    Never,
    RetrySameRequest,
    RetryAfterStateChange,
    NewSessionRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorTerminalScope {
    Request,
    Open,
    Turn,
    SessionReadOnly,
    SessionDeleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalErrorRegistryEntry {
    pub code: CanonicalErrorCode,
    pub version: u32,
    pub class: CanonicalErrorClass,
    pub retry: ErrorRetryDirective,
    pub terminal_scope: ErrorTerminalScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalErrorRegistryPayload {
    pub registry_version: VersionString,
    pub entries: Vec<CanonicalErrorRegistryEntry>,
}

impl CanonicalErrorRegistryPayload {
    pub fn validate(&self) -> Result<(), CanonicalErrorRegistryValidationError> {
        let mut codes = BTreeSet::new();

        for entry in &self.entries {
            let code = entry.code.as_ref();
            if !is_upper_snake_case(code) {
                return Err(CanonicalErrorRegistryValidationError::InvalidCode {
                    code: code.to_owned(),
                });
            }
            if !codes.insert(code.to_owned()) {
                return Err(CanonicalErrorRegistryValidationError::DuplicateCode {
                    code: code.to_owned(),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CanonicalErrorRegistryValidationError {
    #[error("canonical error code must be upper snake case: {code}")]
    InvalidCode { code: String },
    #[error("duplicate canonical error code: {code}")]
    DuplicateCode { code: String },
}

fn is_upper_snake_case(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('_')
        && !value.ends_with('_')
        && !value.contains("__")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}
