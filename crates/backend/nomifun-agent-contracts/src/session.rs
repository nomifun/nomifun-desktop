use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::preset::{AgentBindingValue, ResolvedSnapshotRef};
use crate::primitives::{
    AgentSessionId, ArtifactId, CanonicalErrorCode, DigestHex, EventId, LogicalArtifactRef,
    OperationId, PrincipalRef, RemoteBindingId, RuntimeBindingId, VersionString,
};

pub type SessionPayloadId = ArtifactId;
pub type UnixTimestampMs = i64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "record_kind", content = "record", rename_all = "snake_case")]
pub enum AgentSessionAggregate {
    Live(AgentSessionLiveRecord),
    Deleting(AgentSessionDeletingRecord),
    Tombstone(AgentSessionTombstone),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionLiveRecord {
    pub agent_session_id: AgentSessionId,
    pub owner_ref: PrincipalRef,
    pub metadata: AgentSessionMetadata,
    pub agent_binding: AgentBindingValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_binding_provenance: Option<RemoteBindingProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<AgentSessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_base_payload_id: Option<SessionPayloadId>,
    pub next_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionDeletingRecord {
    pub live: AgentSessionLiveRecord,
    pub delete_operation_id: OperationId,
    pub admission_fenced_at: UnixTimestampMs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionTombstone {
    pub agent_session_id: AgentSessionId,
    pub owner_ref: PrincipalRef,
    pub state: AgentSessionDeletedState,
    pub deleted_at: UnixTimestampMs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionDeletedState {
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub archived: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteBindingProvenance {
    pub remote_binding_id: RemoteBindingId,
    pub binding_version: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionPayloadRecord {
    pub payload_id: SessionPayloadId,
    pub agent_session_id: AgentSessionId,
    pub media_type: String,
    pub byte_len: u64,
    pub digest: DigestHex,
    pub body: SessionPayloadBody,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
pub enum SessionPayloadBody {
    Utf8(String),
    Base64(String),
    Json(crate::primitives::StrictJsonValue),
    ArtifactRef(LogicalArtifactRef),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCheckpointBinding {
    pub runtime_binding_id: RuntimeBindingId,
    pub locator: LogicalArtifactRef,
    pub runtime_bound_event_id: EventId,
    pub protocol_version: VersionString,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub through_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointExactMatch {
    RuntimeBoundEvent,
    ProtocolVersion,
    Snapshot,
    ThroughSeq,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointDiscardReason {
    Missing,
    Corrupt,
    RuntimeBoundEventMismatch,
    ProtocolMismatch,
    SnapshotMismatch,
    ThroughSeqMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointBuildIdentitySource {
    RuntimeBoundEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointRehydrateSource {
    ExactSnapshot,
    LatestCompletedCompaction,
    SubsequentCanonicalEvents,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCheckpointContract {
    pub contract_version: VersionString,
    pub binding: RuntimeCheckpointBinding,
    pub required_exact_matches: Vec<CheckpointExactMatch>,
    pub actual_runtime_build_source: CheckpointBuildIdentitySource,
    pub discard_on: Vec<CheckpointDiscardReason>,
    pub rehydrate_from: Vec<CheckpointRehydrateSource>,
    pub checkpoint_converter_allowed: bool,
    pub incompatible_executor_error: CanonicalErrorCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactionCompletedPayload {
    pub agent_session_id: AgentSessionId,
    pub through_seq: u64,
    pub context_payload_id: SessionPayloadId,
    pub context_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionForkPayload {
    pub parent_session_id: AgentSessionId,
    pub parent_through_seq: u64,
    pub child_session_id: AgentSessionId,
    pub child_base_payload_id: SessionPayloadId,
    pub child_base_digest: DigestHex,
    pub child_agent_binding: AgentBindingValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionForkContract {
    pub contract_version: VersionString,
    pub fork: SessionForkPayload,
    pub child_base_is_self_contained: bool,
    pub copies_full_transcript: bool,
    pub migrates_runtime_private_handles: bool,
    pub replays_tool_or_effect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteAgentSessionCommand {
    pub operation_id: OperationId,
    pub agent_session_id: AgentSessionId,
    pub owner_ref: PrincipalRef,
    pub requested_at: UnixTimestampMs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeleteClosureStep {
    FenceAdmission,
    QuiesceAndCancelRuntime,
    ProveZeroOutstanding,
    PurgeSessionPrivateContent,
    CommitTombstone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeleteOutstandingKind {
    RuntimeBinding,
    RuntimeAck,
    Turn,
    ToolDispatch,
    EffectDispatch,
    CapabilityInstanceHandle,
    ResourceHandle,
    Task,
    DescendantProcess,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionPrivateContentKind {
    SessionEvent,
    SessionPayload,
    SessionHeadProjection,
    MessageProjection,
    Message,
    SessionOwnedAttachment,
    SessionOwnedArtifact,
    RuntimeBinding,
    RuntimeCheckpoint,
    SessionScopedResource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomainEffectDeletionPolicy {
    RetainOwningDomainFactsWithMinimalSourceReference,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionDeleteClosureContract {
    pub contract_version: VersionString,
    pub command: DeleteAgentSessionCommand,
    pub ordered_steps: Vec<DeleteClosureStep>,
    pub required_zero: Vec<DeleteOutstandingKind>,
    pub purge_targets: Vec<SessionPrivateContentKind>,
    pub final_tombstone: AgentSessionTombstone,
    pub final_tombstone_fields: Vec<String>,
    pub late_operation_error: CanonicalErrorCode,
    pub domain_effect_policy: DomainEffectDeletionPolicy,
}
