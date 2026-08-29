use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::{RuntimeEventAck, RuntimeEventEnvelope};
use crate::package::{CapabilityRef, PackageRef, SkillRef};
use crate::preset::ResolvedSnapshotRef;
use crate::session::{CheckpointDiscardReason, RuntimeCheckpointBinding};
use crate::{
    ActionId, AgentSessionId, ArtifactEnvelope, CanonicalDigestError, CanonicalErrorCode,
    CapabilityId, ConnectionConfigRef, DigestHex, EventId, IdempotencyKey, LogicalArtifactRef,
    McpServerId, McpToolKey, ModelRouteId, OperationId, PackageId, PrincipalRef, ResourceBindingId,
    RuntimeBindingId, RuntimeFeatureId, RuntimeTarget, SkillId, TypedResourceBindings,
    VersionString, digest_payload,
};

pub const FROZEN_CODEX_INVESTIGATION_SHA: &str =
    "dc2ccc6843abb09c9d297862dc10b6bd12a3935d";
pub const OBSERVED_CODEX_SIBLING_HEAD_SHA: &str =
    "4ee04c0aa5833ac39b1763f6ea44c7bc777c83dd";
pub const OBSERVED_CODEX_COMMITS_AHEAD: u32 = 16;
pub const SNAPSHOT_EXECUTOR_UNAVAILABLE_CODE: &str = "SNAPSHOT_EXECUTOR_UNAVAILABLE";
pub const SNAPSHOT_EXECUTOR_UNAVAILABLE: &str = SNAPSHOT_EXECUTOR_UNAVAILABLE_CODE;
pub const CODEX_BASELINE_PIN_MISMATCH: &str = "CODEX_BASELINE_PIN_MISMATCH";
pub const CODEX_BASELINE_DRIFT_MISMATCH: &str = "CODEX_BASELINE_DRIFT_MISMATCH";
pub const RUNTIME_RPC_ALLOWLIST_MISMATCH: &str = "RUNTIME_RPC_ALLOWLIST_MISMATCH";
pub const RUNTIME_PROFILE_SET_MISMATCH: &str = "RUNTIME_PROFILE_SET_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodingRuntimeFeatureInventoryPayload {
    pub schema_version: VersionString,
    pub pinned_source: CodexPinnedSource,
    pub supported_profiles: BTreeSet<RuntimeProfileKind>,
    pub runtime_features: BTreeSet<RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub responses_semantics: BTreeSet<String>,
    pub rpc_allowlist: RuntimeRpcAllowlist,
    pub full_auto: FullAutoExecutionWire,
}

impl CodingRuntimeFeatureInventoryPayload {
    pub fn validate(&self) -> Result<(), RuntimeContractViolation> {
        self.pinned_source.validate_frozen_investigation_baseline()?;
        if self.supported_profiles
            != BTreeSet::from([
                RuntimeProfileKind::CodingNative,
                RuntimeProfileKind::ManagedMinimal,
            ])
        {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(RUNTIME_PROFILE_SET_MISMATCH),
                message: "runtime feature inventory must expose exactly two profiles".to_owned(),
            });
        }
        if !self.rpc_allowlist.is_frozen_exact_set() || self.full_auto != FullAutoExecutionWire::fixed()
        {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(RUNTIME_RPC_ALLOWLIST_MISMATCH),
                message: "runtime feature inventory must use the frozen RPC and FullAuto contract"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContractViolation {
    pub code: CanonicalErrorCode,
    pub message: String,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfileKind {
    CodingNative,
    ManagedMinimal,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum FullAutoAskForApproval {
    Never,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum FullAutoSandboxPolicy {
    DangerFullAccess,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FullAutoExecutionWire {
    pub ask_for_approval: FullAutoAskForApproval,
    pub sandbox_policy: FullAutoSandboxPolicy,
}

impl FullAutoExecutionWire {
    pub const fn fixed() -> Self {
        Self {
            ask_for_approval: FullAutoAskForApproval::Never,
            sandbox_policy: FullAutoSandboxPolicy::DangerFullAccess,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAuthorityCheckKind {
    PrincipalOwnership,
    SnapshotCapabilityAllowlist,
    TypedResourceBinding,
    RemoteIngressAuthentication,
    ProviderCredentialCentralStorage,
}

pub const RUNTIME_AUTHORITY_CHECK_ORDER: [RuntimeAuthorityCheckKind; 5] = [
    RuntimeAuthorityCheckKind::PrincipalOwnership,
    RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
    RuntimeAuthorityCheckKind::TypedResourceBinding,
    RuntimeAuthorityCheckKind::RemoteIngressAuthentication,
    RuntimeAuthorityCheckKind::ProviderCredentialCentralStorage,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAuthorityContract {
    pub ordered_checks: [RuntimeAuthorityCheckKind; 5],
}

impl RuntimeAuthorityContract {
    pub const fn fixed() -> Self {
        Self {
            ordered_checks: RUNTIME_AUTHORITY_CHECK_ORDER,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAuthorityInput {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub active_set_generation: u64,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
    pub remote_ingress_authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_connection_config_ref: Option<ConnectionConfigRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RuntimeAuthorityDecision {
    Allow,
    Deny {
        failed_check: RuntimeAuthorityCheckKind,
        error_code: CanonicalErrorCode,
    },
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRpcMethod {
    Create,
    Resume,
    Fork,
    StartTurn,
    Steer,
    FollowUp,
    Cancel,
    SessionDispose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRpcAllowlist {
    pub methods: BTreeSet<RuntimeRpcMethod>,
    pub experimental_methods: BTreeSet<String>,
}

impl RuntimeRpcAllowlist {
    pub fn frozen() -> Self {
        Self {
            methods: BTreeSet::from([
                RuntimeRpcMethod::Create,
                RuntimeRpcMethod::Resume,
                RuntimeRpcMethod::Fork,
                RuntimeRpcMethod::StartTurn,
                RuntimeRpcMethod::Steer,
                RuntimeRpcMethod::FollowUp,
                RuntimeRpcMethod::Cancel,
                RuntimeRpcMethod::SessionDispose,
            ]),
            experimental_methods: BTreeSet::new(),
        }
    }

    pub fn is_frozen_exact_set(&self) -> bool {
        self == &Self::frozen()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHelloPayload {
    pub runtime_release_digest: DigestHex,
    pub runtime_build_digest: DigestHex,
    pub fork_commit: String,
    pub tracked_upstream_commit: String,
    pub protocol_version: VersionString,
    pub protocol_schema_digest: DigestHex,
    pub runtime_target: RuntimeTarget,
    pub supported_profiles: BTreeSet<RuntimeProfileKind>,
    pub native_features: BTreeSet<RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub full_auto: FullAutoExecutionWire,
    pub rpc_allowlist: RuntimeRpcAllowlist,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCommandContext {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub operation_id: OperationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub runtime_profile_digest: DigestHex,
    pub active_set_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBindingContract {
    pub runtime_binding_id: RuntimeBindingId,
    pub agent_session_id: AgentSessionId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub runtime_release_digest: DigestHex,
    pub runtime_build_digest: DigestHex,
    pub protocol_version: VersionString,
    pub profile_kind: RuntimeProfileKind,
    pub runtime_profile_digest: DigestHex,
    pub active_set_generation: u64,
    pub runtime_bound_event_id: EventId,
    pub through_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCreateParams {
    pub context: RuntimeCommandContext,
    pub profile_kind: RuntimeProfileKind,
    pub full_auto: FullAutoExecutionWire,
    pub initial_capabilities: BTreeSet<CapabilityId>,
    pub on_demand_capabilities: BTreeSet<CapabilityId>,
    pub typed_resource_bindings: TypedResourceBindings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeResumeParams {
    pub context: RuntimeCommandContext,
    pub compatibility_admission_input_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<RuntimeCheckpointBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeForkParams {
    pub source_agent_session_id: AgentSessionId,
    pub child_context: RuntimeCommandContext,
    pub source_through_seq: u64,
    pub self_contained_fork_base_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeStartTurnParams {
    pub context: RuntimeCommandContext,
    pub idempotency_key: IdempotencyKey,
    pub input_event_id: EventId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSteerParams {
    pub context: RuntimeCommandContext,
    pub target_turn_operation_id: OperationId,
    pub input_event_id: EventId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFollowUpParams {
    pub context: RuntimeCommandContext,
    pub idempotency_key: IdempotencyKey,
    pub input_event_id: EventId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCancelParams {
    pub context: RuntimeCommandContext,
    pub target_operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSessionDisposeParams {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub operation_id: OperationId,
    pub reason: CanonicalErrorCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RuntimeCommand {
    Create(RuntimeCreateParams),
    Resume(RuntimeResumeParams),
    Fork(RuntimeForkParams),
    StartTurn(RuntimeStartTurnParams),
    Steer(RuntimeSteerParams),
    FollowUp(RuntimeFollowUpParams),
    Cancel(RuntimeCancelParams),
    SessionDispose(RuntimeSessionDisposeParams),
}

pub type RuntimeEventWireEnvelope = RuntimeEventEnvelope;
pub type RuntimeEventWireAck = RuntimeEventAck;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEventResendRequest {
    pub runtime_binding_id: RuntimeBindingId,
    pub next_producer_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEventResendBatch {
    pub request: RuntimeEventResendRequest,
    pub envelopes: Vec<RuntimeEventWireEnvelope>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeActionStart {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub turn_operation_id: OperationId,
    pub action_id: ActionId,
    pub effect_id: EventId,
    pub idempotency_key: IdempotencyKey,
    pub capability_id: CapabilityId,
    pub active_set_generation: u64,
    pub snapshot_digest: DigestHex,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeActionStartAck {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub turn_operation_id: OperationId,
    pub action_id: ActionId,
    pub effect_id: EventId,
    pub idempotency_key: IdempotencyKey,
    pub capability_id: CapabilityId,
    pub active_set_generation: u64,
    pub snapshot_digest: DigestHex,
    pub effect_started_event_id: EventId,
    pub committed_session_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeActionStartAckExchange {
    pub start: NativeActionStart,
    pub ack: NativeActionStartAck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCheckpointValidationInput {
    pub checkpoint: RuntimeCheckpointBinding,
    pub referenced_runtime_build_digest: DigestHex,
    pub expected_runtime_bound_event_id: EventId,
    pub expected_runtime_build_digest: DigestHex,
    pub expected_protocol_version: VersionString,
    pub expected_snapshot_ref: ResolvedSnapshotRef,
    pub expected_through_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RuntimeCheckpointValidationResult {
    ExactMatch,
    Mismatch {
        mismatches: Vec<CheckpointDiscardReason>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCheckpointMismatchFixture {
    pub input: RuntimeCheckpointValidationInput,
    pub result: RuntimeCheckpointValidationResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimePackageExecutionContract {
    pub exact_package: PackageRef,
    pub manifest_digest: DigestHex,
    pub execution_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilityExecutionContract {
    pub exact_capability: CapabilityRef,
    pub schema_digest: DigestHex,
    pub implementation_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSkillExecutionContract {
    pub exact_skill: SkillRef,
    pub body_digest: DigestHex,
    pub required_capability_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMcpToolExecutionContract {
    pub capability_id: CapabilityId,
    pub server_id: McpServerId,
    pub canonical_tool_key: McpToolKey,
    pub schema_digest: DigestHex,
    pub materialization_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeModelRouteExecutionContract {
    pub model_route_id: ModelRouteId,
    pub config_revision_digest: DigestHex,
    pub protocol_contract_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecutionCeiling {
    pub protocol_version: VersionString,
    pub protocol_schema_digest: DigestHex,
    pub profile_kind: RuntimeProfileKind,
    pub profile_digest: DigestHex,
    pub native_features: BTreeSet<RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub initial_capabilities: BTreeMap<CapabilityId, RuntimeCapabilityExecutionContract>,
    pub on_demand_capabilities: BTreeMap<CapabilityId, RuntimeCapabilityExecutionContract>,
    pub packages: BTreeMap<PackageId, RuntimePackageExecutionContract>,
    pub skills: BTreeMap<SkillId, RuntimeSkillExecutionContract>,
    pub mcp_tools: BTreeMap<CapabilityId, RuntimeMcpToolExecutionContract>,
    pub model_routes: BTreeMap<ModelRouteId, RuntimeModelRouteExecutionContract>,
    pub typed_resource_bindings: TypedResourceBindings,
    pub typed_resource_contract_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecutorSupport {
    pub runtime_release_digest: DigestHex,
    pub hello_payload_digest: DigestHex,
    pub protocol_versions: BTreeSet<VersionString>,
    pub protocol_schema_digests: BTreeSet<DigestHex>,
    pub profile_digests: BTreeMap<RuntimeProfileKind, BTreeSet<DigestHex>>,
    pub native_features: BTreeSet<RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub capabilities: BTreeMap<CapabilityId, RuntimeCapabilityExecutionContract>,
    pub packages: BTreeMap<PackageId, RuntimePackageExecutionContract>,
    pub skills: BTreeMap<SkillId, RuntimeSkillExecutionContract>,
    pub mcp_tools: BTreeMap<CapabilityId, RuntimeMcpToolExecutionContract>,
    pub model_routes: BTreeMap<ModelRouteId, RuntimeModelRouteExecutionContract>,
    pub typed_resource_contract_digests: BTreeSet<DigestHex>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCompatibilityAdmissionInput {
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub required_ceiling: RuntimeExecutionCeiling,
    pub available_executor: RuntimeExecutorSupport,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotContractMismatchKind {
    ProtocolVersion,
    ProtocolSchema,
    RuntimeProfile,
    NativeFeature,
    NativeAction,
    InitialCapability,
    OnDemandCapability,
    Package,
    Skill,
    McpTool,
    ModelRoute,
    TypedResourceContract,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContractMismatch {
    pub kind: SnapshotContractMismatchKind,
    pub subject: String,
    pub expected: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SnapshotCompatibilityAdmissionResult {
    CompatibleExact {
        runtime_release_digest: DigestHex,
        hello_payload_digest: DigestHex,
    },
    ExecutorUnavailable {
        error_code: CanonicalErrorCode,
        mismatches: Vec<SnapshotContractMismatch>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodexPinnedSource {
    pub repository_alias: String,
    pub pinned_commit: String,
}

impl CodexPinnedSource {
    pub fn frozen_investigation_baseline() -> Self {
        Self {
            repository_alias: "../codex".to_owned(),
            pinned_commit: FROZEN_CODEX_INVESTIGATION_SHA.to_owned(),
        }
    }

    pub fn validate_frozen_investigation_baseline(
        &self,
    ) -> Result<(), RuntimeContractViolation> {
        if self.repository_alias == "../codex"
            && self.pinned_commit == FROZEN_CODEX_INVESTIGATION_SHA
        {
            Ok(())
        } else {
            Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(CODEX_BASELINE_PIN_MISMATCH),
                message: "runtime release source must pin the frozen ../codex investigation commit"
                    .to_owned(),
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodexBaselineDriftPayload {
    pub release_pin: CodexPinnedSource,
    pub observed_sibling_head: String,
    pub observed_commits_ahead: u32,
    pub frozen_commit_is_ancestor: bool,
}

pub type CodexBaselineDriftArtifact = ArtifactEnvelope<CodexBaselineDriftPayload>;

impl CodexBaselineDriftPayload {
    pub fn observed_checkout() -> Self {
        Self {
            release_pin: CodexPinnedSource::frozen_investigation_baseline(),
            observed_sibling_head: OBSERVED_CODEX_SIBLING_HEAD_SHA.to_owned(),
            observed_commits_ahead: OBSERVED_CODEX_COMMITS_AHEAD,
            frozen_commit_is_ancestor: true,
        }
    }

    pub fn validate(&self) -> Result<(), RuntimeContractViolation> {
        self.release_pin.validate_frozen_investigation_baseline()?;
        if self.observed_sibling_head == OBSERVED_CODEX_SIBLING_HEAD_SHA
            && self.observed_commits_ahead == OBSERVED_CODEX_COMMITS_AHEAD
            && self.frozen_commit_is_ancestor
        {
            Ok(())
        } else {
            Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(CODEX_BASELINE_DRIFT_MISMATCH),
                message:
                    "Codex sibling drift must record the observed descendant without changing the release pin"
                        .to_owned(),
            })
        }
    }

    pub fn payload_digest(&self) -> Result<DigestHex, CanonicalDigestError> {
        digest_payload(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "support", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeReleaseTargetPayload {
    Required {
        host_target: RuntimeTarget,
        runtime_target: RuntimeTarget,
        package_format: String,
        host_artifact: LogicalArtifactRef,
        sidecar_artifact: LogicalArtifactRef,
        helper_artifacts: Vec<LogicalArtifactRef>,
        package_content_digest: DigestHex,
        capability_availability_digest: DigestHex,
    },
    Unsupported {
        capability_availability_digest: DigestHex,
    },
    RemoteOnly {
        capability_availability_digest: DigestHex,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodexRuntimeReleaseManifestPayload {
    pub manifest_version: VersionString,
    pub pinned_source: CodexPinnedSource,
    pub fork_commit: String,
    pub tracked_upstream_commit: String,
    pub patch_series_digest: DigestHex,
    pub cargo_lock_digest: DigestHex,
    pub protocol_version: VersionString,
    pub protocol_schema_digest: DigestHex,
    pub rpc_allowlist: RuntimeRpcAllowlist,
    pub full_auto: FullAutoExecutionWire,
    pub supported_profiles: BTreeSet<RuntimeProfileKind>,
    pub runtime_profile_contract_digest: DigestHex,
    pub coding_capability_pack_digest: DigestHex,
    pub native_feature_contract_digest: DigestHex,
    pub native_action_contract_digest: DigestHex,
    pub target_matrix: BTreeMap<String, RuntimeReleaseTargetPayload>,
    pub license_artifact: LogicalArtifactRef,
    pub notice_artifact: LogicalArtifactRef,
    pub sbom_artifact: LogicalArtifactRef,
}

pub type CodexRuntimeReleaseManifest = ArtifactEnvelope<CodexRuntimeReleaseManifestPayload>;

impl CodexRuntimeReleaseManifestPayload {
    pub fn validate(&self) -> Result<(), RuntimeContractViolation> {
        self.pinned_source.validate_frozen_investigation_baseline()?;
        if self.tracked_upstream_commit != FROZEN_CODEX_INVESTIGATION_SHA {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(CODEX_BASELINE_PIN_MISMATCH),
                message: "tracked_upstream_commit must equal the frozen investigation commit"
                    .to_owned(),
            });
        }
        if self.fork_commit == OBSERVED_CODEX_SIBLING_HEAD_SHA {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(CODEX_BASELINE_PIN_MISMATCH),
                message: "the observed sibling HEAD cannot be used as the pinned release fork"
                    .to_owned(),
            });
        }
        if !self.rpc_allowlist.is_frozen_exact_set() {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(RUNTIME_RPC_ALLOWLIST_MISMATCH),
                message: "runtime RPC methods must equal the frozen eight-method allowlist"
                    .to_owned(),
            });
        }
        let expected_profiles = BTreeSet::from([
            RuntimeProfileKind::CodingNative,
            RuntimeProfileKind::ManagedMinimal,
        ]);
        if self.supported_profiles != expected_profiles {
            return Err(RuntimeContractViolation {
                code: CanonicalErrorCode::from(RUNTIME_PROFILE_SET_MISMATCH),
                message: "runtime profiles must be exactly coding_native and managed_minimal"
                    .to_owned(),
            });
        }
        Ok(())
    }

    pub fn runtime_release_digest(&self) -> Result<DigestHex, CanonicalDigestError> {
        digest_payload(self)
    }
}
