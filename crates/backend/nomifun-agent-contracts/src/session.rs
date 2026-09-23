use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::preset::{AgentBindingValue, PresetRevisionRef, ResolvedSnapshotRef};
use crate::primitives::{
    AgentSessionId, ArtifactId, CanonicalErrorCode, DigestHex, EventId, LogicalArtifactRef,
    OperationId, PrincipalRef, RemoteBindingId, RuntimeBindingId, VersionString,
};

pub type SessionPayloadId = ArtifactId;
pub type UnixTimestampMs = i64;

pub const AGENT_HANDOFF_ENVELOPE_SCHEMA_V1: &str = "1";
pub const MAX_AGENT_HANDOFF_ENVELOPE_BYTES: usize = 80 * 1024;

/// Explicit user choice for one Agent binding transition. The first release
/// never guesses task continuation from transcript contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHandoffMode {
    ContinueTask,
    ContextOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffBindingRefV1 {
    pub preset_revision_ref: PresetRevisionRef,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub binding_version: u64,
}

impl From<&AgentBindingValue> for AgentHandoffBindingRefV1 {
    fn from(binding: &AgentBindingValue) -> Self {
        Self {
            preset_revision_ref: binding.preset_revision_ref.clone(),
            resolved_snapshot_ref: binding.resolved_snapshot_ref.clone(),
            binding_version: binding.binding_version,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffInputCitationV1 {
    pub input: usize,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffRequirementOriginV1 {
    pub turn_operation_id: String,
    pub requirement_id: String,
    pub source: AgentHandoffInputCitationV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffRequirementV1 {
    pub id: String,
    pub description: String,
    pub source: AgentHandoffInputCitationV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<AgentHandoffRequirementOriginV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffPlanStepV1 {
    pub step: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffPlanV1 {
    pub revision: u16,
    pub explanation: String,
    pub steps: Vec<AgentHandoffPlanStepV1>,
    pub needs_replan: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffCompletionCriterionV1 {
    pub step: String,
    pub disposition: String,
    pub evidence_call_ids: Vec<String>,
    pub rationale: String,
    pub requirement_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_change: Option<AgentHandoffInputCitationV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffCompletionAccountV1 {
    pub plan_revision: u16,
    pub observation_revision: u32,
    pub workspace_epoch: u32,
    pub summary: String,
    pub criteria: Vec<AgentHandoffCompletionCriterionV1>,
}

/// A verified historical workspace artifact emitted by the exact source Turn.
/// It is a data reference only: the target Agent must re-read and re-verify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffVerifiedArtifactV1 {
    pub artifact_id: String,
    pub source_path: String,
    pub relative_path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Bounded deterministic task facts exported from the latest exact closed Turn.
/// This payload never contains prompts, tool arguments/results, effects,
/// credentials, private handles, checkpoints, processes, or capability grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentHandoffEnvelopeV1 {
    pub schema_version: String,
    pub source_agent_session_id: AgentSessionId,
    pub source_turn_operation_id: OperationId,
    pub source_through_seq: u64,
    pub source_binding_ref: AgentHandoffBindingRefV1,
    pub target_binding_ref: AgentHandoffBindingRefV1,
    pub mode: AgentHandoffMode,
    /// V1 deliberately keeps historical requirements out of the target
    /// completion gate until a real target-Turn input establishes provenance.
    pub completion_gate_inherited: bool,
    #[serde(default)]
    pub requirements: Vec<AgentHandoffRequirementV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_plan: Option<AgentHandoffPlanV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_completion_account: Option<AgentHandoffCompletionAccountV1>,
    #[serde(default)]
    pub verified_artifacts: Vec<AgentHandoffVerifiedArtifactV1>,
    #[serde(default)]
    pub unresolved_items: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl AgentHandoffEnvelopeV1 {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != AGENT_HANDOFF_ENVELOPE_SCHEMA_V1 {
            return Err("unsupported Agent handoff schema version".to_owned());
        }
        if self.mode != AgentHandoffMode::ContinueTask {
            return Err("a persisted Agent handoff envelope requires continue_task mode".to_owned());
        }
        if self.source_turn_operation_id.as_ref().trim().is_empty() || self.source_through_seq == 0 {
            return Err("Agent handoff requires an exact closed source Turn".to_owned());
        }
        if self.completion_gate_inherited {
            return Err("Agent handoff v1 cannot inherit the source completion gate".to_owned());
        }
        if self.requirements.len() > 32
            || self.last_plan.as_ref().is_some_and(|plan| plan.steps.len() > 16)
            || self
                .historical_completion_account
                .as_ref()
                .is_some_and(|account| account.criteria.len() > 16)
            || self.verified_artifacts.len() > 32
            || self.unresolved_items.len() > 32
            || self.warnings.len() > 32
        {
            return Err("Agent handoff collection budget exceeded".to_owned());
        }
        let bounded = |value: &str, maximum: usize| {
            !value.trim().is_empty() && value.len() <= maximum && value == value.trim()
        };
        let mut requirement_ids = BTreeSet::new();
        let mut artifact_ids = BTreeSet::new();
        if self.requirements.iter().any(|item| {
            !bounded(&item.id, 64)
                || !requirement_ids.insert(item.id.as_str())
                || !bounded(&item.description, 512)
                || item.source.quote.len() > 512
                || item.origin.as_ref().is_some_and(|origin| {
                    !bounded(&origin.turn_operation_id, 1024)
                        || !bounded(&origin.requirement_id, 64)
                        || origin.source.quote.len() > 512
                })
            })
            || self.last_plan.as_ref().is_some_and(|plan| {
                plan.revision == 0
                    || plan.explanation.len() > 2048
                    || plan.steps.iter().any(|step| {
                        !bounded(&step.step, 512)
                            || !matches!(
                                step.status.as_str(),
                                "pending" | "in_progress" | "completed" | "blocked"
                            )
                    })
            })
            || self
                .historical_completion_account
                .as_ref()
                .is_some_and(|account| {
                    account.summary.len() > 2048
                        || account.criteria.iter().any(|criterion| {
                            !bounded(&criterion.step, 512)
                                || !matches!(
                                    criterion.disposition.as_str(),
                                    "supported" | "unverified" | "blocked" | "scope_changed"
                                )
                                || !bounded(&criterion.rationale, 1024)
                                || criterion.evidence_call_ids.len() > 8
                                || criterion.requirement_ids.len() > 32
                        })
                })
            || self.verified_artifacts.iter().any(|item| {
                !bounded(&item.artifact_id, 256)
                    || !artifact_ids.insert(item.artifact_id.as_str())
                    || !bounded(&item.source_path, 4096)
                    || !bounded(&item.relative_path, 4096)
                    || !bounded(&item.mime_type, 256)
                    || item.sha256.len() != 64
                    || !item
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            || self
                .unresolved_items
                .iter()
                .chain(self.warnings.iter())
                .any(|item| !bounded(item, 2048))
        {
            return Err("Agent handoff contains an invalid bounded field".to_owned());
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("Agent handoff is not serializable: {error}"))?;
        if bytes.len() > MAX_AGENT_HANDOFF_ENVELOPE_BYTES {
            return Err(format!(
                "Agent handoff exceeds {MAX_AGENT_HANDOFF_ENVELOPE_BYTES} bytes"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingChangedPayloadV1 {
    pub transition_id: OperationId,
    pub request_digest: DigestHex,
    pub previous_binding_ref: AgentHandoffBindingRefV1,
    pub next_binding_ref: AgentHandoffBindingRefV1,
    pub previous_agent_label: String,
    pub next_agent_label: String,
    pub handoff_mode: AgentHandoffMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_payload_id: Option<SessionPayloadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_payload_digest: Option<DigestHex>,
    pub completion_gate_inherited: bool,
    pub effective_after_seq: u64,
}

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

#[cfg(test)]
mod agent_handoff_tests {
    use super::*;
    use crate::{AgentPresetId, ResolvedSnapshotId};

    fn binding(version: u64, suffix: &str) -> AgentHandoffBindingRefV1 {
        AgentHandoffBindingRefV1 {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from(format!("preset-{suffix}")),
                revision: version,
                revision_digest: DigestHex::from("a".repeat(64)),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from(format!("snapshot-{suffix}")),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
            binding_version: version,
        }
    }

    fn envelope() -> AgentHandoffEnvelopeV1 {
        AgentHandoffEnvelopeV1 {
            schema_version: AGENT_HANDOFF_ENVELOPE_SCHEMA_V1.to_owned(),
            source_agent_session_id: AgentSessionId::from(
                "0190f5fe-7c00-7a00-8000-000000000001",
            ),
            source_turn_operation_id: OperationId::from("turn-1"),
            source_through_seq: 8,
            source_binding_ref: binding(1, "source"),
            target_binding_ref: binding(2, "target"),
            mode: AgentHandoffMode::ContinueTask,
            completion_gate_inherited: false,
            requirements: vec![AgentHandoffRequirementV1 {
                id: "req-1".to_owned(),
                description: "Preserve the same Session".to_owned(),
                source: AgentHandoffInputCitationV1 {
                    input: 0,
                    quote: "same Session".to_owned(),
                },
                origin: None,
            }],
            last_plan: None,
            historical_completion_account: None,
            verified_artifacts: Vec::new(),
            unresolved_items: vec!["Re-verify the current workspace".to_owned()],
            warnings: vec![
                "Historical requirements are data only and are not in the target completion gate"
                    .to_owned(),
            ],
        }
    }

    #[test]
    fn handoff_v1_is_bounded_and_rejects_unknown_fields() {
        let envelope = envelope();
        envelope.validate().unwrap();
        let mut wire = serde_json::to_value(envelope).unwrap();
        wire.as_object_mut()
            .unwrap()
            .insert("old_system_prompt".to_owned(), serde_json::json!("forbidden"));
        assert!(serde_json::from_value::<AgentHandoffEnvelopeV1>(wire).is_err());
    }

    #[test]
    fn handoff_v1_never_inherits_the_completion_gate_or_exceeds_budget() {
        let mut inherited = envelope();
        inherited.completion_gate_inherited = true;
        assert!(inherited.validate().is_err());

        let mut oversized = envelope();
        oversized.warnings = vec!["x".repeat(MAX_AGENT_HANDOFF_ENVELOPE_BYTES)];
        assert!(oversized.validate().is_err());
    }
}
