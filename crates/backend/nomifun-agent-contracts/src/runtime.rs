//! Shared Kernel authority and persisted snapshot/checkpoint compatibility.
//! The retired external executor's hello, command and native-action wire
//! protocol is deliberately absent. RuntimeProfileKind below is a persisted
//! snapshot compatibility tag, not the list of registered Engine families.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::package::{CapabilityRef, PackageRef, SkillRef};
use crate::preset::ResolvedSnapshotRef;
use crate::session::{CheckpointDiscardReason, RuntimeCheckpointBinding};
use crate::{
    ActionId, AgentSessionId, CanonicalErrorCode, CapabilityId, ConnectionConfigRef, DigestHex,
    EventId, McpServerId, McpToolKey, ModelRouteId, PackageId, PrincipalRef, ResourceBindingId,
    RuntimeFeatureId, SkillId, TypedResourceBindings, VersionString,
};

pub const SNAPSHOT_EXECUTOR_UNAVAILABLE_CODE: &str = "SNAPSHOT_EXECUTOR_UNAVAILABLE";
pub const SNAPSHOT_EXECUTOR_UNAVAILABLE: &str = SNAPSHOT_EXECUTOR_UNAVAILABLE_CODE;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContractViolation {
    pub code: CanonicalErrorCode,
    pub message: String,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfileKind {
    CodingNative,
    ManagedMinimal,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
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
    pub enabled_capabilities: BTreeMap<CapabilityId, RuntimeCapabilityExecutionContract>,
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
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotContractMismatchKind {
    ProtocolVersion,
    ProtocolSchema,
    RuntimeProfile,
    NativeFeature,
    NativeAction,
    EnabledCapability,
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
