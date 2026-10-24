use std::collections::BTreeSet;

pub const MAX_NATIVE_EXECUTION_CHECKPOINT_BYTES: usize = 256 * 1024;
/// Complete model-authored completion arguments, including the final answer,
/// criteria, references and JSON escaping. This is a serialized UTF-8 byte
/// budget, not a separate character limit on the delivered answer.
pub const MAX_AGENT_COMPLETION_REPORT_BYTES: usize = 48 * 1024;
// Writer, recovery and closed-history readers share these ceilings. A
// segment rollover never makes a Turn too large for its own recovery codec.
pub const MAX_NATIVE_JOURNAL_WINDOW_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_NATIVE_JOURNAL_PROGRESS_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_NATIVE_JOURNAL_TOTAL_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_NATIVE_JOURNAL_RECORDS: u64 = 64_000;
pub const MAX_NATIVE_REPLAY_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_NATIVE_HISTORY_WINDOW_BYTES: usize = 160 * 1024 * 1024;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::preset::{AgentBindingValue, PresetRevisionRef, ResolvedSnapshotRef};
use crate::primitives::{
    AgentSessionId, ArtifactId, CanonicalErrorCode, DigestHex, LogicalArtifactRef,
    OperationId, PrincipalRef, RemoteBindingId, VersionString,
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

/// Exact existing binding and active-set authority at one canonical clear.
/// Resource and model replacement advance the referenced binding version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextClearAdmissionProofV1 {
    pub binding_ref: AgentHandoffBindingRefV1,
    pub active_set_generation: u64,
}

/// The original clear command remains replayable for reason-only facts from
/// this generation. New writers always include admission; its absence cannot
/// authorize a product input that has not yet been accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextClearedPayloadV1 {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission: Option<ContextClearAdmissionProofV1>,
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
    pub revision: u32,
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
    pub plan_revision: u32,
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
        let bounded_text = |value: &str, maximum: usize| {
            !value.trim().is_empty() && value.chars().count() <= maximum && value == value.trim()
        };
        let mut requirement_ids = BTreeSet::new();
        let mut artifact_ids = BTreeSet::new();
        if self.requirements.iter().any(|item| {
            !bounded(&item.id, 64)
                || !requirement_ids.insert(item.id.as_str())
                || !bounded_text(&item.description, 512)
                || item.source.quote.chars().count() > 512
                || item.origin.as_ref().is_some_and(|origin| {
                    !bounded(&origin.turn_operation_id, 1024)
                        || !bounded(&origin.requirement_id, 64)
                        || origin.source.quote.chars().count() > 512
                })
            })
            || self.last_plan.as_ref().is_some_and(|plan| {
                plan.revision == 0
                    || plan.explanation.chars().count() > 2048
                    || plan.steps.iter().any(|step| {
                        !bounded_text(&step.step, 512)
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
                    account.criteria.iter().any(|criterion| {
                        !bounded_text(&criterion.step, 512)
                            || !matches!(
                                criterion.disposition.as_str(),
                                "supported" | "unverified" | "blocked" | "scope_changed"
                            )
                            || !bounded_text(&criterion.rationale, 1024)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_relocation: Option<crate::ManagedStorageRelocationV1>,
}

impl AgentBindingChangedPayloadV1 {
    pub fn validate_storage_relocation(&self) -> Result<(), String> {
        let proof = self.workspace_relocation.as_ref().ok_or("storage transition proof is missing")?;
        proof.validate()?;
        if crate::digest_payload(proof).map_err(|error| error.to_string())? != self.request_digest {
            return Err("storage transition request digest differs from its exact proof".into());
        }
        if self.previous_binding_ref.preset_revision_ref != self.next_binding_ref.preset_revision_ref
            || self.previous_binding_ref.resolved_snapshot_ref != self.next_binding_ref.resolved_snapshot_ref
            || self.previous_binding_ref.binding_version.checked_add(1) != Some(self.next_binding_ref.binding_version)
            || self.previous_agent_label != self.next_agent_label
            || self.handoff_mode != AgentHandoffMode::ContextOnly || self.completion_gate_inherited
            || self.handoff_payload_id.is_some() || self.handoff_payload_digest.is_some()
        { return Err("storage transition changed the frozen Agent or inherited execution state".into()); }
        Ok(())
    }
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

/// Immutable product ownership established by the canonical opening event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionPurpose {
    #[default]
    Conversation,
    /// Typed recognition of existing history only. Current product owners
    /// reject new creation, execution, forks and checkpoint recovery.
    PluginAuthoring,
}

impl SessionPurpose {
    pub fn is_conversation(&self) -> bool { *self == Self::Conversation }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompanionSessionRole {
    Companion,
    Robot,
}

/// Immutable product origin from the canonical Session opening. Resources and
/// their current grants remain in the frozen Agent binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompanionSessionOrigin {
    pub companion_id: String,
    pub role: CompanionSessionRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robot_id: Option<String>,
}

/// Trusted ingress evidence attached by the authenticated host, rather than
/// inferred from user text or mutable message projections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentInputSource {
    AuthenticatedRobot {
        companion_id: String,
        robot_id: String,
        connection_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presented_question_id: Option<String>,
    },

}

/// Server-authored source of one scheduled invocation. It is carried by the
/// original canonical accepted input after the host validated the exact run
/// reservation; user message fields and operation spellings cannot mint it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CronRunRuntimeSource {
    pub cron_job_id: String,
    pub cron_job_run_id: String,
    pub agent_session_id: AgentSessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionMetadata {
    #[serde(default, skip_serializing_if = "SessionPurpose::is_conversation")]
    pub purpose: SessionPurpose,
    /// Immutable application project affiliation from this canonical opening.
    /// It does not grant resource access or change the ordinary Runtime contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_project_id: Option<String>,
    /// Companion identity and ingress role are fixed at creation, independent
    /// of later Agent/model/resource transitions and mutable UI projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion: Option<CompanionSessionOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub archived: bool,
    pub pinned: bool,
    /// Session-owned override. None inherits the selected model's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<crate::ReasoningEffort>,
}

#[cfg(test)]
mod session_purpose_tests {
    use super::*;

    #[test]
    fn current_generation_metadata_defaults_to_conversation_and_preserves_authoring_purpose() {
        let ordinary = serde_json::json!({"title":"Chat","archived":false,"pinned":false});
        let metadata: AgentSessionMetadata = serde_json::from_value(ordinary.clone()).unwrap();
        assert_eq!(metadata.purpose, SessionPurpose::Conversation);
        assert!(metadata.companion.is_none());
        assert_eq!(serde_json::to_value(metadata).unwrap(), ordinary);
        let authoring = serde_json::json!({"title":"App","archived":false,"pinned":false,"purpose":"plugin_authoring"});
        let metadata: AgentSessionMetadata = serde_json::from_value(authoring.clone()).unwrap();
        assert_eq!(metadata.purpose, SessionPurpose::PluginAuthoring);
        assert_eq!(serde_json::to_value(metadata).unwrap(), authoring);
        for purpose in [serde_json::json!("unknown"), serde_json::Value::Null] {
            let mut invalid = ordinary.clone(); invalid["purpose"] = purpose;
            assert!(serde_json::from_value::<AgentSessionMetadata>(invalid).is_err());
        }
    }

    #[test]
    fn companion_origin_has_explicit_roles_and_strict_fields() {
        let origin = serde_json::json!({
            "companion_id": "019abcab-cdef-7000-8000-000000000000",
            "role": "robot",
            "robot_id": "robot-board-1",
        });
        let parsed: CompanionSessionOrigin = serde_json::from_value(origin.clone()).unwrap();
        assert_eq!(parsed.role, CompanionSessionRole::Robot);
        assert_eq!(serde_json::to_value(parsed).unwrap(), origin);
        let mut unknown = origin.clone();
        unknown["channel"] = serde_json::json!("desktop");
        assert!(serde_json::from_value::<CompanionSessionOrigin>(unknown).is_err());
        let mut invalid_role = origin;
        invalid_role["role"] = serde_json::json!("desktop");
        assert!(serde_json::from_value::<CompanionSessionOrigin>(invalid_role).is_err());
    }

    #[test]
    fn authenticated_robot_source_is_typed_and_rejects_unknown_evidence() {
        let source = serde_json::json!({
            "kind": "authenticated_robot",
            "companion_id": "019abcab-cdef-7000-8000-000000000000",
            "robot_id": "robot-board-1",
            "connection_id": "connection-1",
        });
        let parsed: AgentInputSource = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), source);
        let mut missing = source.clone();
        missing.as_object_mut().unwrap().remove("connection_id");
        assert!(serde_json::from_value::<AgentInputSource>(missing).is_err());
        let mut unknown = source;
        unknown["trusted"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentInputSource>(unknown).is_err());
    }
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
    fn long_task_revisions_and_unicode_fields_survive_handoff_serialization() {
        let mut value = envelope();
        value.requirements[0].description = "需求".repeat(256);
        value.requirements[0].source.quote = "来源".repeat(256);
        value.last_plan = Some(AgentHandoffPlanV1 {
            revision: 65_536,
            explanation: "解释".repeat(1024),
            steps: vec![AgentHandoffPlanStepV1 { step: "步骤".repeat(256), status: "completed".into() }],
            needs_replan: false,
        });
        value.historical_completion_account = Some(AgentHandoffCompletionAccountV1 {
            plan_revision: 65_536, observation_revision: 1, workspace_epoch: 0,
            summary: "完整交付总结".repeat(1024),
            criteria: vec![AgentHandoffCompletionCriterionV1 {
                step: "步骤".repeat(256), disposition: "unverified".into(),
                rationale: "原因".repeat(512), evidence_call_ids: vec![],
                requirement_ids: vec!["req-1".into()], scope_change: None,
            }],
        });
        value.validate().unwrap();
        let restored: AgentHandoffEnvelopeV1 = serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, value);
        assert!(restored.historical_completion_account.as_ref().unwrap().summary.chars().count()>2048,
            "explicit Agent continuation preserves the full accepted answer within the original envelope budget");
        value.requirements[0].source.quote.push('超');
        assert!(value.validate().is_err());
    }

    #[test]
    fn complete_handoff_answer_remains_bounded_by_serialized_utf8_envelope() {
        let mut value=envelope();
        value.historical_completion_account=Some(AgentHandoffCompletionAccountV1 {
            plan_revision:1,observation_revision:1,workspace_epoch:0,
            summary:"界".repeat(MAX_AGENT_HANDOFF_ENVELOPE_BYTES/3+1),criteria:vec![],
        });
        assert!(value.historical_completion_account.as_ref().unwrap().summary.chars().count()<MAX_AGENT_HANDOFF_ENVELOPE_BYTES);
        let error=value.validate().unwrap_err();
        assert!(error.contains(&format!("Agent handoff exceeds {MAX_AGENT_HANDOFF_ENVELOPE_BYTES} bytes")),"{error}");
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
