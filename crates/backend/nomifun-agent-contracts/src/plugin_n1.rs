//! Phase N1 Plugin machine contracts.
//!
//! These types freeze immutable artifact, runtime, project, candidate, test,
//! operation, and platform-evidence shapes. They contain no loader, process,
//! database, or product-service implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ActionId, ArtifactEnvelope, ArtifactId, CandidateTestReceiptId,
    CanonicalErrorCode, CanonicalSchemaRef, CapabilityRef,
    ContributionId, CorrelationId, CredentialId, CredentialSlotKey, DigestHex,
    LogicalArtifactRef, MiniAppId, OperationId,
    PackageContributions, PackageEntrypointMetadata, PackageManifest, PackageRef,
    PluginCandidateId, PluginMountId, PluginProjectId,
    PluginStateCompareAndSwapOutcome,
    PluginStateCompareAndSwapRequest, PluginStateDeleteRequest,
    PluginStateDeleteResponse, PluginStateGetRequest, PluginStateGetResponse,
    PluginStateHandleDescriptor, PluginStateSetRequest, PluginStateSetResponse,
    ResourceBindingId, ResourceKind, RuntimeInstallationId, RuntimeTarget,
    StrictJsonValue, ValidatedPluginConfig, ValidationCohortId, VersionString,
    digest_payload,
};

pub const PLUGIN_N1_SCHEMA_VERSION: &str = "1.0.0";
pub const PLUGIN_PACKAGE_PROFILE_VERSION: &str = "1.0.0";
pub const JAVASCRIPT_HOST_PROTOCOL_VERSION: &str = "1.0.0";
pub const JAVASCRIPT_SDK_CONTRACT_VERSION: &str = "1.0.0";
pub const CANDIDATE_TEST_CONTRACT_VERSION: &str = "1.0.0";
pub const MINIMUM_NODE_MAJOR: u16 = 22;
pub const RECOMMENDED_NODE_LTS_MAJOR: u16 = 24;

pub type PluginN1ContractArtifact = ArtifactEnvelope<PluginN1ContractManifest>;

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
pub enum JavaScriptBuildProfile {
    PluginPackageV1,
    MiniAppReleaseV1,
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
pub enum JavaScriptHostMethod {
    MountLoad,
    MountUnload,
    CapabilityInvoke,
    ContextContribute,
    ResourceAcquire,
    ResourceRelease,
    CredentialResolve,
    StateGet,
    StateSet,
    StateDelete,
    StateCompareAndSwap,
    BuildExecute,
    RequestCancel,
    HostShutdown,
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
pub enum JavaScriptHostMessageDirection {
    HostToJavaScript,
    JavaScriptToHost,
}

impl JavaScriptHostMethod {
    pub const EXTENSION_HOST: [Self; 13] = [
        Self::MountLoad,
        Self::MountUnload,
        Self::CapabilityInvoke,
        Self::ContextContribute,
        Self::ResourceAcquire,
        Self::ResourceRelease,
        Self::CredentialResolve,
        Self::StateGet,
        Self::StateSet,
        Self::StateDelete,
        Self::StateCompareAndSwap,
        Self::RequestCancel,
        Self::HostShutdown,
    ];

    pub const CANDIDATE_TEST_HOST: [Self; 13] = Self::EXTENSION_HOST;

    pub const BUILD_HOST: [Self; 3] = [
        Self::BuildExecute,
        Self::RequestCancel,
        Self::HostShutdown,
    ];
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
pub enum ProductOperationKind {
    Build,
    Import,
    Export,
    MiniappPermanentDelete,
}

impl ProductOperationKind {
    pub const DURABLE: [Self; 4] = [
        Self::Build,
        Self::Import,
        Self::Export,
        Self::MiniappPermanentDelete,
    ];
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
pub enum ProductOperationState {
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl ProductOperationState {
    pub const ALL: [Self; 4] = [
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Canceled,
    ];

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
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
pub enum N1PlatformCellId {
    WindowsDesktopX64,
    MacosDesktopArm64,
    LinuxDesktopX64,
    MacosDesktopX64,
    LinuxHeadlessX64,
}

impl N1PlatformCellId {
    pub const REQUIRED: [Self; 3] = [
        Self::WindowsDesktopX64,
        Self::MacosDesktopArm64,
        Self::LinuxDesktopX64,
    ];
    pub const OPTIONAL: [Self; 2] =
        [Self::MacosDesktopX64, Self::LinuxHeadlessX64];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginN1ContractManifest {
    pub schema_version: VersionString,
    pub plugin_package_profile_version: VersionString,
    pub javascript_host_protocol_version: VersionString,
    pub javascript_sdk_contract_version: VersionString,
    pub candidate_test_contract_version: VersionString,
    pub minimum_node_major: u16,
    pub recommended_node_lts_major: u16,
    pub host_method_sets:
        BTreeMap<JavaScriptHostKind, BTreeSet<JavaScriptHostMethod>>,
    pub durable_operation_kinds: BTreeSet<ProductOperationKind>,
    pub durable_operation_states: BTreeSet<ProductOperationState>,
    pub required_cells: BTreeSet<N1PlatformCellId>,
    pub optional_cells: BTreeSet<N1PlatformCellId>,
}

impl PluginN1ContractManifest {
    pub fn canonical() -> Self {
        Self {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            plugin_package_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            javascript_host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            candidate_test_contract_version: CANDIDATE_TEST_CONTRACT_VERSION.into(),
            minimum_node_major: MINIMUM_NODE_MAJOR,
            recommended_node_lts_major: RECOMMENDED_NODE_LTS_MAJOR,
            host_method_sets: BTreeMap::from([
                (
                    JavaScriptHostKind::SharedExtension,
                    JavaScriptHostMethod::EXTENSION_HOST.into_iter().collect(),
                ),
                (
                    JavaScriptHostKind::CandidateTest,
                    JavaScriptHostMethod::CANDIDATE_TEST_HOST
                        .into_iter()
                        .collect(),
                ),
                (
                    JavaScriptHostKind::Build,
                    JavaScriptHostMethod::BUILD_HOST.into_iter().collect(),
                ),
            ]),
            durable_operation_kinds: ProductOperationKind::DURABLE.into_iter().collect(),
            durable_operation_states: ProductOperationState::ALL.into_iter().collect(),
            required_cells: N1PlatformCellId::REQUIRED.into_iter().collect(),
            optional_cells: N1PlatformCellId::OPTIONAL.into_iter().collect(),
        }
    }

    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        let canonical = Self::canonical();
        if self != &canonical {
            return Err(PluginN1ContractError::InvalidField {
                field: "plugin_n1_contract",
                reason: "manifest differs from the frozen Phase N1 exact set".into(),
            });
        }
        Ok(())
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
pub enum NodeRuntimeSourceKind {
    ManualPath,
    ProcessPath,
    Managed,
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
pub enum NodeProbeDisposition {
    CompatibleRecommended,
    CompatibleNonRecommended,
    Incompatible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeRuntimeFingerprint {
    pub runtime_installation_id: RuntimeInstallationId,
    pub source_kind: NodeRuntimeSourceKind,
    pub node_version: VersionString,
    pub node_major: u16,
    pub runtime_target: RuntimeTarget,
    pub executable_digest: DigestHex,
    pub javascript_host_protocol_version: VersionString,
    pub javascript_sdk_contract_version: VersionString,
}

impl NodeRuntimeFingerprint {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(
            self.runtime_installation_id.as_ref(),
            "runtime_installation_id",
        )?;
        validate_nonempty(self.node_version.as_ref(), "node_version")?;
        validate_nonempty(self.runtime_target.as_ref(), "runtime_target")?;
        validate_digest(&self.executable_digest, "executable_digest")?;
        if self.node_major < MINIMUM_NODE_MAJOR {
            return Err(PluginN1ContractError::InvalidField {
                field: "node_major",
                reason: format!("Node {MINIMUM_NODE_MAJOR}+ is required"),
            });
        }
        require_version(
            &self.javascript_host_protocol_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "javascript_host_protocol_version",
        )?;
        require_version(
            &self.javascript_sdk_contract_version,
            JAVASCRIPT_SDK_CONTRACT_VERSION,
            "javascript_sdk_contract_version",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeRuntimeProbeResult {
    pub source_kind: NodeRuntimeSourceKind,
    pub executable_path: String,
    pub disposition: NodeProbeDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<NodeRuntimeFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<CanonicalErrorCode>,
}

impl NodeRuntimeProbeResult {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(&self.executable_path, "executable_path")?;
        match self.disposition {
            NodeProbeDisposition::CompatibleRecommended
            | NodeProbeDisposition::CompatibleNonRecommended => {
                validate_absolute_path(&self.executable_path, "executable_path")?;
                let fingerprint = self.fingerprint.as_ref().ok_or_else(|| {
                    PluginN1ContractError::InvalidField {
                        field: "fingerprint",
                        reason: "compatible probe requires a runtime fingerprint".into(),
                    }
                })?;
                fingerprint.validate()?;
                if fingerprint.source_kind != self.source_kind {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "source_kind",
                        reason:
                            "probe and fingerprint source kinds must be identical"
                                .into(),
                    });
                }
                if self.error_code.is_some() {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "error_code",
                        reason: "compatible probe cannot carry an error".into(),
                    });
                }
                let is_recommended =
                    fingerprint.node_major == RECOMMENDED_NODE_LTS_MAJOR;
                if is_recommended
                    != (self.disposition
                        == NodeProbeDisposition::CompatibleRecommended)
                {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "disposition",
                        reason:
                            "recommended disposition must match the frozen LTS major"
                                .into(),
                    });
                }
            }
            NodeProbeDisposition::Incompatible => {
                if self.fingerprint.is_some() || self.error_code.is_none() {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "probe_result",
                        reason:
                            "incompatible probe requires an error and no fingerprint"
                                .into(),
                    });
                }
            }
        }
        Ok(())
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
pub enum RuntimeSwitchParticipantKind {
    PluginMount,
    MiniappService,
    BuildFoundation,
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
pub enum RuntimeSwitchParticipantOutcome {
    Passed,
    Failed,
    NotCovered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSwitchParticipantResult {
    pub kind: RuntimeSwitchParticipantKind,
    pub owner_id: String,
    pub outcome: RuntimeSwitchParticipantOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<CanonicalErrorCode>,
}

impl RuntimeSwitchParticipantResult {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(&self.owner_id, "runtime_switch.owner_id")?;
        if (self.outcome == RuntimeSwitchParticipantOutcome::Failed)
            != self.error_code.is_some()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "runtime_switch.error_code",
                reason: "only failed participants carry an error code".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSwitchValidationResult {
    pub candidate: NodeRuntimeFingerprint,
    pub foundation_hello_passed: bool,
    pub old_runtime_process_tree_zero: bool,
    pub participants: Vec<RuntimeSwitchParticipantResult>,
    pub completed_at_ms: i64,
}

impl RuntimeSwitchValidationResult {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        self.candidate.validate()?;
        if !self.foundation_hello_passed || !self.old_runtime_process_tree_zero
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "runtime_switch.foundation",
                reason: "switch validation requires foundation Hello and zero old processes"
                    .into(),
            });
        }
        if self.completed_at_ms <= 0 {
            return Err(PluginN1ContractError::InvalidField {
                field: "runtime_switch.completed_at_ms",
                reason: "completion time must be positive".into(),
            });
        }
        let mut owners = BTreeSet::new();
        for participant in &self.participants {
            participant.validate()?;
            if !owners.insert((participant.kind, participant.owner_id.as_str())) {
                return Err(PluginN1ContractError::DuplicateIdentity {
                    field: "runtime_switch.participant",
                    value: participant.owner_id.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn requires_user_decision(&self) -> bool {
        self.participants.iter().any(|participant| {
            participant.outcome != RuntimeSwitchParticipantOutcome::Passed
        })
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
pub enum RuntimeSwitchDecision {
    CommitCandidate,
    AbortAndRestoreSelected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSelectionRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_runtime: Option<NodeRuntimeFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_candidate: Option<NodeRuntimeFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_result: Option<RuntimeSwitchValidationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<CanonicalErrorCode>,
    pub non_recommended_warning_acknowledged:
        BTreeSet<RuntimeInstallationId>,
}

impl RuntimeSelectionRecord {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        if let Some(selected) = &self.selected_runtime {
            selected.validate()?;
        }
        if let Some(candidate) = &self.pending_candidate {
            candidate.validate()?;
        }
        if let Some(result) = &self.validation_result {
            result.validate()?;
            if self.pending_candidate.as_ref() != Some(&result.candidate) {
                return Err(PluginN1ContractError::InvalidField {
                    field: "validation_result",
                    reason:
                        "switch validation must bind the exact pending candidate"
                            .into(),
                });
            }
        }
        if self.pending_candidate.is_none() && self.validation_result.is_some() {
            return Err(PluginN1ContractError::InvalidField {
                field: "pending_candidate",
                reason: "validation cannot outlive its pending candidate".into(),
            });
        }
        Ok(())
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
pub enum JavaScriptHostKind {
    SharedExtension,
    CandidateTest,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JavaScriptHostHello {
    pub schema_version: VersionString,
    pub host_kind: JavaScriptHostKind,
    pub host_generation: u64,
    pub process_id: u32,
    pub runtime: NodeRuntimeFingerprint,
    pub supported_methods: BTreeSet<JavaScriptHostMethod>,
}

impl JavaScriptHostHello {
    pub fn validate(
        &self,
        contract: &PluginN1ContractManifest,
    ) -> Result<(), PluginN1ContractError> {
        contract.validate()?;
        require_version(&self.schema_version, PLUGIN_N1_SCHEMA_VERSION, "schema_version")?;
        if self.host_generation == 0 || self.process_id == 0 {
            return Err(PluginN1ContractError::InvalidField {
                field: "host_identity",
                reason: "host_generation and process_id must be non-zero".into(),
            });
        }
        self.runtime.validate()?;
        let expected = contract
            .host_method_sets
            .get(&self.host_kind)
            .ok_or_else(|| PluginN1ContractError::InvalidField {
                field: "host_kind",
                reason: "Host kind is not part of the frozen N1 contract".into(),
            })?;
        if &self.supported_methods != expected {
            return Err(PluginN1ContractError::InvalidField {
                field: "supported_methods",
                reason:
                    "Host Hello method set differs from its role-specific contract"
                        .into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SensitiveString(pub String);

impl fmt::Debug for SensitiveString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveString([REDACTED])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHostTargetLock {
    pub mount_id: PluginMountId,
    pub package: PackageRef,
    pub artifact_digest: DigestHex,
    pub manifest_digest: DigestHex,
}

impl PluginHostTargetLock {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.mount_id.as_ref(), "target.mount_id")?;
        validate_nonempty(self.package.id.as_ref(), "target.package.id")?;
        validate_nonempty(self.package.version.as_ref(), "target.package.version")?;
        validate_digest(&self.artifact_digest, "target.artifact_digest")?;
        validate_digest(&self.manifest_digest, "target.manifest_digest")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHostContributionRef {
    pub target: PluginHostTargetLock,
    pub contribution_id: ContributionId,
    pub capability: CapabilityRef,
    pub contract_digest: DigestHex,
}

impl PluginHostContributionRef {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        self.target.validate()?;
        validate_nonempty(
            self.contribution_id.as_ref(),
            "contribution.contribution_id",
        )?;
        validate_nonempty(
            self.capability.id.as_ref(),
            "contribution.capability.id",
        )?;
        validate_nonempty(
            self.capability.version.as_ref(),
            "contribution.capability.version",
        )?;
        validate_digest(
            &self.contract_digest,
            "contribution.contract_digest",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginMountRuntimeContext {
    pub target: PluginHostTargetLock,
    pub mount_handle_id: String,
    pub config: ValidatedPluginConfig,
    pub credential_bindings: Vec<CredentialSlotBinding>,
    pub state: PluginStateHandleDescriptor,
    pub data_dir: String,
}

impl PluginMountRuntimeContext {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        self.target.validate()?;
        validate_nonempty(&self.mount_handle_id, "mount_handle_id")?;
        validate_absolute_path(&self.data_dir, "data_dir")?;
        if self.state.mount_id != self.target.mount_id
            || self.state.package_id != self.target.package.id
            || self.state.methods
                != crate::PluginStateMethod::REQUIRED.into_iter().collect()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "state",
                reason:
                    "state handle must bind the exact target Mount and four-method contract"
                        .into(),
            });
        }
        let mut slots = BTreeSet::new();
        for binding in &self.credential_bindings {
            binding.validate_for_mount(&self.target.mount_id)?;
            if !slots.insert(binding.slot_key.clone()) {
                return Err(PluginN1ContractError::DuplicateIdentity {
                    field: "credential_bindings.slot_key",
                    value: binding.slot_key.as_ref().to_owned(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum PluginHostRequest {
    MountLoad {
        context: PluginMountRuntimeContext,
    },
    MountUnload {
        target: PluginHostTargetLock,
    },
    CapabilityInvoke {
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    },
    ContextContribute {
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    },
    ResourceAcquire {
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    },
    ResourceRelease {
        handle_id: String,
    },
    CredentialResolve {
        mount_handle_id: String,
        slot_key: CredentialSlotKey,
    },
    StateGet {
        mount_handle_id: String,
        request: PluginStateGetRequest,
    },
    StateSet {
        mount_handle_id: String,
        request: PluginStateSetRequest,
    },
    StateDelete {
        mount_handle_id: String,
        request: PluginStateDeleteRequest,
    },
    StateCompareAndSwap {
        mount_handle_id: String,
        request: PluginStateCompareAndSwapRequest,
    },
    BuildExecute {
        profile: JavaScriptBuildProfile,
        source_root: String,
        staging_root: String,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
    },
    RequestCancel {
        target_request_id: CorrelationId,
    },
    HostShutdown,
}

impl PluginHostRequest {
    pub const fn method(&self) -> JavaScriptHostMethod {
        match self {
            Self::MountLoad { .. } => JavaScriptHostMethod::MountLoad,
            Self::MountUnload { .. } => JavaScriptHostMethod::MountUnload,
            Self::CapabilityInvoke { .. } => {
                JavaScriptHostMethod::CapabilityInvoke
            }
            Self::ContextContribute { .. } => {
                JavaScriptHostMethod::ContextContribute
            }
            Self::ResourceAcquire { .. } => {
                JavaScriptHostMethod::ResourceAcquire
            }
            Self::ResourceRelease { .. } => {
                JavaScriptHostMethod::ResourceRelease
            }
            Self::CredentialResolve { .. } => {
                JavaScriptHostMethod::CredentialResolve
            }
            Self::StateGet { .. } => JavaScriptHostMethod::StateGet,
            Self::StateSet { .. } => JavaScriptHostMethod::StateSet,
            Self::StateDelete { .. } => JavaScriptHostMethod::StateDelete,
            Self::StateCompareAndSwap { .. } => {
                JavaScriptHostMethod::StateCompareAndSwap
            }
            Self::BuildExecute { .. } => JavaScriptHostMethod::BuildExecute,
            Self::RequestCancel { .. } => JavaScriptHostMethod::RequestCancel,
            Self::HostShutdown => JavaScriptHostMethod::HostShutdown,
        }
    }

    pub const fn direction(&self) -> JavaScriptHostMessageDirection {
        match self {
            Self::CredentialResolve { .. }
            | Self::StateGet { .. }
            | Self::StateSet { .. }
            | Self::StateDelete { .. }
            | Self::StateCompareAndSwap { .. } => {
                JavaScriptHostMessageDirection::JavaScriptToHost
            }
            Self::MountLoad { .. }
            | Self::MountUnload { .. }
            | Self::CapabilityInvoke { .. }
            | Self::ContextContribute { .. }
            | Self::ResourceAcquire { .. }
            | Self::ResourceRelease { .. }
            | Self::BuildExecute { .. }
            | Self::RequestCancel { .. }
            | Self::HostShutdown => {
                JavaScriptHostMessageDirection::HostToJavaScript
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHostRequestEnvelope {
    pub protocol_version: VersionString,
    pub host_kind: JavaScriptHostKind,
    pub host_generation: u64,
    pub request_id: CorrelationId,
    pub direction: JavaScriptHostMessageDirection,
    pub request: PluginHostRequest,
}

impl PluginHostRequestEnvelope {
    pub fn validate(
        &self,
        contract: &PluginN1ContractManifest,
    ) -> Result<(), PluginN1ContractError> {
        contract.validate()?;
        require_version(
            &self.protocol_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "protocol_version",
        )?;
        if self.host_generation == 0 {
            return Err(PluginN1ContractError::InvalidField {
                field: "host_generation",
                reason: "host generation must be non-zero".into(),
            });
        }
        validate_nonempty(self.request_id.as_ref(), "request_id")?;
        let allowed = contract
            .host_method_sets
            .get(&self.host_kind)
            .ok_or_else(|| PluginN1ContractError::InvalidField {
                field: "host_kind",
                reason: "unknown Host role".into(),
            })?;
        if !allowed.contains(&self.request.method()) {
            return Err(PluginN1ContractError::InvalidField {
                field: "request.method",
                reason: "request is not allowed for this Host role".into(),
            });
        }
        if self.direction != self.request.direction() {
            return Err(PluginN1ContractError::InvalidField {
                field: "direction",
                reason: "message direction does not match the requested Host method".into(),
            });
        }
        validate_host_request(&self.request)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum PluginHostSuccess {
    Ack,
    Value(StrictJsonValue),
    ResourceAcquired {
        handle_id: String,
    },
    CredentialResolved {
        slot_key: CredentialSlotKey,
        secret: SensitiveString,
    },
    StateGet(PluginStateGetResponse),
    StateSet(PluginStateSetResponse),
    StateDelete(PluginStateDeleteResponse),
    StateCompareAndSwap(PluginStateCompareAndSwapOutcome),
    BuildCompleted {
        artifact_id: ArtifactId,
        artifact_digest: DigestHex,
        manifest_digest: DigestHex,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHostWireError {
    pub code: CanonicalErrorCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", content = "value", rename_all = "snake_case")]
pub enum PluginHostResponseBody {
    Success(PluginHostSuccess),
    Failure(PluginHostWireError),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHostResponseEnvelope {
    pub protocol_version: VersionString,
    pub host_kind: JavaScriptHostKind,
    pub host_generation: u64,
    pub request_id: CorrelationId,
    pub response: PluginHostResponseBody,
}

impl PluginHostResponseEnvelope {
    pub fn validate_for(
        &self,
        request: &PluginHostRequestEnvelope,
    ) -> Result<(), PluginN1ContractError> {
        require_version(
            &self.protocol_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "protocol_version",
        )?;
        if self.host_kind != request.host_kind
            || self.host_generation != request.host_generation
            || self.request_id != request.request_id
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "response_identity",
                reason:
                    "response must bind the exact Host role, generation, and request"
                        .into(),
            });
        }
        validate_host_response(request, &self.response)?;
        Ok(())
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
pub enum CredentialSlotKind {
    SecretText,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialSlotDeclaration {
    pub slot_key: CredentialSlotKey,
    pub kind: CredentialSlotKind,
    pub display_name: String,
    pub required: bool,
}

impl CredentialSlotDeclaration {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_machine_key(self.slot_key.as_ref(), "credential_slots.slot_key")?;
        validate_nonempty(&self.display_name, "credential_slots.display_name")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialSlotBinding {
    pub mount_id: PluginMountId,
    pub slot_key: CredentialSlotKey,
    pub credential_id: CredentialId,
}

impl CredentialSlotBinding {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.mount_id.as_ref(), "mount_id")?;
        validate_machine_key(self.slot_key.as_ref(), "slot_key")?;
        validate_nonempty(self.credential_id.as_ref(), "credential_id")
    }

    pub fn validate_for_mount(
        &self,
        mount_id: &PluginMountId,
    ) -> Result<(), PluginN1ContractError> {
        self.validate()?;
        if &self.mount_id != mount_id {
            return Err(PluginN1ContractError::InvalidField {
                field: "mount_id",
                reason: "credential binding belongs to another Mount".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactFileDigest {
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
}

impl ArtifactFileDigest {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_relative_path(&self.normalized_relative_path, "artifact_file.path")?;
        validate_digest(&self.digest, "artifact_file.digest")?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPackageV1Manifest {
    pub schema_version: VersionString,
    pub build_profile: JavaScriptBuildProfile,
    pub build_profile_version: VersionString,
    pub package: PackageManifest,
    pub supported_targets: BTreeSet<RuntimeTarget>,
    pub minimum_node_major: u16,
    pub dependency_lock_digest: DigestHex,
    pub credential_slots: Vec<CredentialSlotDeclaration>,
}

impl PluginPackageV1Manifest {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        require_version(&self.schema_version, PLUGIN_N1_SCHEMA_VERSION, "schema_version")?;
        if self.build_profile != JavaScriptBuildProfile::PluginPackageV1 {
            return Err(PluginN1ContractError::InvalidField {
                field: "build_profile",
                reason: "Plugin artifacts must use plugin_package_v1".into(),
            });
        }
        require_version(
            &self.build_profile_version,
            PLUGIN_PACKAGE_PROFILE_VERSION,
            "build_profile_version",
        )?;
        require_version(
            &self.package.schema_version,
            PLUGIN_N1_SCHEMA_VERSION,
            "package.schema_version",
        )?;
        validate_nonempty(self.package.package_id.as_ref(), "package.package_id")?;
        validate_nonempty(
            self.package.package_version.as_ref(),
            "package.package_version",
        )?;
        validate_nonempty(&self.package.display.name, "package.display.name")?;
        validate_nonempty(
            &self.package.display.description,
            "package.display.description",
        )?;
        require_version(
            &self.package.host_contract_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "package.host_contract_version",
        )?;
        let entrypoint = match &self.package.entrypoint {
            PackageEntrypointMetadata::JavaScript(entrypoint) => entrypoint,
            PackageEntrypointMetadata::InProcess(_) => {
                return Err(PluginN1ContractError::InvalidField {
                    field: "package.entrypoint",
                    reason: "Plugin Package v1 requires a JavaScript entrypoint".into(),
                });
            }
        };
        if entrypoint.normalized_relative_path != "main.mjs" {
            return Err(PluginN1ContractError::InvalidField {
                field: "package.entrypoint.normalized_relative_path",
                reason: "Plugin Package v1 entrypoint must be main.mjs".into(),
            });
        }
        validate_digest(&entrypoint.module_digest, "package.entrypoint.module_digest")?;
        require_version(
            &entrypoint.host_protocol_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "package.entrypoint.host_protocol_version",
        )?;
        require_version(
            &entrypoint.sdk_contract_version,
            JAVASCRIPT_SDK_CONTRACT_VERSION,
            "package.entrypoint.sdk_contract_version",
        )?;
        if self.package.host_contract_version != entrypoint.host_protocol_version {
            return Err(PluginN1ContractError::InvalidField {
                field: "package.entrypoint.host_protocol_version",
                reason: "entrypoint and Package Host contracts must match".into(),
            });
        }
        if !self.package.package_dependencies.is_empty()
            || !self.package.provides_services.is_empty()
            || !self.package.requires_services.is_empty()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "package",
                reason: "Plugin Package v1 has no Plugin dependency DAG or Plugin Service"
                    .into(),
            });
        }
        if !self.package.config_schema.0.is_object() {
            return Err(PluginN1ContractError::InvalidField {
                field: "package.config_schema",
                reason: "config_schema must be a JSON object".into(),
            });
        }
        if self.supported_targets.is_empty() {
            return Err(PluginN1ContractError::InvalidField {
                field: "supported_targets",
                reason: "at least one host target is required".into(),
            });
        }
        if self.minimum_node_major < MINIMUM_NODE_MAJOR {
            return Err(PluginN1ContractError::InvalidField {
                field: "minimum_node_major",
                reason: format!("Node {MINIMUM_NODE_MAJOR}+ is required"),
            });
        }
        validate_digest(&self.dependency_lock_digest, "dependency_lock_digest")?;
        let mut slots = BTreeSet::new();
        for slot in &self.credential_slots {
            slot.validate()?;
            if !slots.insert(slot.slot_key.clone()) {
                return Err(PluginN1ContractError::DuplicateIdentity {
                    field: "credential_slots.slot_key",
                    value: slot.slot_key.as_ref().to_owned(),
                });
            }
        }
        validate_package_contributions(
            &PackageRef {
                id: self.package.package_id.clone(),
                version: self.package.package_version.clone(),
            },
            &self.package.contributions,
        )?;
        Ok(())
    }

    pub fn package_ref(&self) -> PackageRef {
        PackageRef {
            id: self.package.package_id.clone(),
            version: self.package.package_version.clone(),
        }
    }

    pub fn entrypoint_digest(&self) -> &DigestHex {
        match &self.package.entrypoint {
            PackageEntrypointMetadata::JavaScript(entrypoint) => {
                &entrypoint.module_digest
            }
            PackageEntrypointMetadata::InProcess(_) => {
                unreachable!("validated Plugin Package v1 entrypoint")
            }
        }
    }
}

#[derive(Serialize)]
struct PluginPackageArtifactDigestInput<'a> {
    manifest: &'a ArtifactEnvelope<PluginPackageV1Manifest>,
    files: &'a [ArtifactFileDigest],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPackageArtifactV1 {
    pub artifact_id: ArtifactId,
    pub artifact_digest: DigestHex,
    pub manifest: ArtifactEnvelope<PluginPackageV1Manifest>,
    pub files: Vec<ArtifactFileDigest>,
}

impl PluginPackageArtifactV1 {
    pub fn new(
        artifact_id: ArtifactId,
        manifest: PluginPackageV1Manifest,
        mut files: Vec<ArtifactFileDigest>,
    ) -> Result<Self, PluginN1ContractError> {
        manifest.validate()?;
        files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        let manifest = ArtifactEnvelope::new(manifest)?;
        let artifact_digest = digest_payload(&PluginPackageArtifactDigestInput {
            manifest: &manifest,
            files: &files,
        })?;
        let value = Self {
            artifact_id,
            artifact_digest,
            manifest,
            files,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.artifact_id.as_ref(), "artifact_id")?;
        validate_digest(&self.artifact_digest, "artifact_digest")?;
        if !self.manifest.verify()? {
            return Err(PluginN1ContractError::DigestMismatch {
                field: "manifest.payload_digest",
            });
        }
        self.manifest.payload.validate()?;
        if self.files.is_empty() {
            return Err(PluginN1ContractError::InvalidField {
                field: "files",
                reason: "Plugin artifact must contain main.mjs".into(),
            });
        }
        let mut previous: Option<&str> = None;
        let mut main_digest = None;
        for file in &self.files {
            file.validate()?;
            if previous.is_some_and(|value| value >= file.normalized_relative_path.as_str()) {
                return Err(PluginN1ContractError::InvalidField {
                    field: "files",
                    reason: "artifact files must be sorted and unique".into(),
                });
            }
            let path = file.normalized_relative_path.as_str();
            if path != "main.mjs"
                && path != "main.mjs.map"
                && !path.starts_with("resources/")
            {
                return Err(PluginN1ContractError::InvalidField {
                    field: "files.normalized_relative_path",
                    reason: "Plugin Package v1 allows main.mjs, main.mjs.map, and resources/** only"
                        .into(),
                });
            }
            if path == "main.mjs" {
                if file.size_bytes == 0 {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "files.main.mjs",
                        reason: "Plugin entrypoint must not be empty".into(),
                    });
                }
                main_digest = Some(&file.digest);
            }
            previous = Some(path);
        }
        if main_digest != Some(self.manifest.payload.entrypoint_digest()) {
            return Err(PluginN1ContractError::DigestMismatch {
                field: "entrypoint.digest",
            });
        }
        let expected = digest_payload(&PluginPackageArtifactDigestInput {
            manifest: &self.manifest,
            files: &self.files,
        })?;
        if expected != self.artifact_digest {
            return Err(PluginN1ContractError::DigestMismatch {
                field: "artifact_digest",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginTargetRef {
    pub package: PackageRef,
    pub artifact_id: ArtifactId,
    pub artifact_digest: DigestHex,
    pub manifest_digest: DigestHex,
}

impl PluginTargetRef {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.package.id.as_ref(), "target.package.id")?;
        validate_nonempty(self.package.version.as_ref(), "target.package.version")?;
        validate_nonempty(self.artifact_id.as_ref(), "target.artifact_id")?;
        validate_digest(&self.artifact_digest, "target.artifact_digest")?;
        validate_digest(&self.manifest_digest, "target.manifest_digest")
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
pub enum PluginCandidateOrigin {
    Build,
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginSourceLineage {
    Managed {
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_profile_version: VersionString,
    },
    RuntimeOnly,
}

impl PluginSourceLineage {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        match self {
            Self::Managed {
                source_snapshot_digest,
                dependency_lock_digest,
                build_profile_version,
            } => {
                validate_digest(source_snapshot_digest, "source_snapshot_digest")?;
                validate_digest(dependency_lock_digest, "dependency_lock_digest")?;
                require_version(
                    build_profile_version,
                    PLUGIN_PACKAGE_PROFILE_VERSION,
                    "build_profile_version",
                )
            }
            Self::RuntimeOnly => Ok(()),
        }
    }

    pub fn is_managed(&self) -> bool {
        matches!(self, Self::Managed { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompatibility {
    Compatible,
    Breaking,
    Unknown,
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
pub enum PluginContractChangeKind {
    ArtifactBytes,
    ContributionSet,
    ContractDigest,
    ConfigSchema,
    CredentialSlots,
    RuntimeRequirement,
    SupportedTargets,
    DependencyLock,
    HostSdkContract,
    ResourceOrEffectContract,
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
pub enum AffectedConsumerKind {
    AgentPresetRevision,
    AgentBinding,
    GatewayOperation,
    RemoteOperation,
    AutomationOperation,
    UiOperation,
    MiniappServiceOperation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AffectedConsumerLockRef {
    pub consumer_kind: AffectedConsumerKind,
    pub consumer_id: String,
    pub mount_id: PluginMountId,
    pub contribution_id: ContributionId,
    pub contract_digest: DigestHex,
}

impl AffectedConsumerLockRef {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(&self.consumer_id, "affected_consumer.consumer_id")?;
        validate_nonempty(
            self.mount_id.as_ref(),
            "affected_consumer.mount_id",
        )?;
        validate_nonempty(
            self.contribution_id.as_ref(),
            "affected_consumer.contribution_id",
        )?;
        validate_digest(
            &self.contract_digest,
            "affected_consumer.contract_digest",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginContractDiff {
    pub compatibility: PluginCompatibility,
    pub changes: BTreeSet<PluginContractChangeKind>,
    pub affected_consumer_locks: Vec<AffectedConsumerLockRef>,
}

impl PluginContractDiff {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        if self.changes.is_empty() {
            return Err(PluginN1ContractError::InvalidField {
                field: "contract_diff.changes",
                reason: "at least artifact_bytes must be recorded".into(),
            });
        }
        if self.compatibility == PluginCompatibility::Compatible
            && self.changes.iter().any(|change| {
                !matches!(
                    change,
                    PluginContractChangeKind::ArtifactBytes
                        | PluginContractChangeKind::DependencyLock
                )
            })
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "contract_diff.compatibility",
                reason: "compatible diff contains a contract-breaking field".into(),
            });
        }
        let mut locks = BTreeSet::new();
        for lock in &self.affected_consumer_locks {
            lock.validate()?;
            if !locks.insert((
                lock.consumer_kind,
                lock.consumer_id.as_str(),
                lock.contribution_id.as_ref(),
            )) {
                return Err(PluginN1ContractError::DuplicateIdentity {
                    field: "affected_consumer_lock",
                    value: lock.consumer_id.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateTestReceiptRef {
    pub receipt_id: CandidateTestReceiptId,
    pub candidate_id: PluginCandidateId,
    pub candidate_digest: DigestHex,
}

impl CandidateTestReceiptRef {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "receipt_id")?;
        validate_nonempty(self.candidate_id.as_ref(), "candidate_id")?;
        validate_digest(&self.candidate_digest, "candidate_digest")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyCandidateRef {
    pub candidate_id: PluginCandidateId,
    pub candidate_digest: DigestHex,
}

impl PluginReadyCandidateRef {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.candidate_id.as_ref(), "candidate_id")?;
        validate_digest(&self.candidate_digest, "candidate_digest")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyCandidate {
    pub candidate_id: PluginCandidateId,
    pub candidate_digest: DigestHex,
    pub project_id: PluginProjectId,
    pub project_build_generation: u64,
    pub origin_operation_id: OperationId,
    pub origin: PluginCandidateOrigin,
    pub target: PluginTargetRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_target_digest: Option<DigestHex>,
    pub source_lineage: PluginSourceLineage,
    pub contract_diff: PluginContractDiff,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_test_receipt: Option<CandidateTestReceiptRef>,
}

#[derive(Serialize)]
struct PluginReadyCandidateDigestInput<'a> {
    project_id: &'a PluginProjectId,
    project_build_generation: u64,
    origin_operation_id: &'a OperationId,
    origin: PluginCandidateOrigin,
    target: &'a PluginTargetRef,
    base_target_digest: &'a Option<DigestHex>,
    source_lineage: &'a PluginSourceLineage,
    contract_diff: &'a PluginContractDiff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyCandidateInput {
    pub project_id: PluginProjectId,
    pub project_build_generation: u64,
    pub origin_operation_id: OperationId,
    pub origin: PluginCandidateOrigin,
    pub target: PluginTargetRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_target_digest: Option<DigestHex>,
    pub source_lineage: PluginSourceLineage,
    pub contract_diff: PluginContractDiff,
}

impl PluginReadyCandidate {
    pub fn new(
        candidate_id: PluginCandidateId,
        input: PluginReadyCandidateInput,
    ) -> Result<Self, PluginN1ContractError> {
        let PluginReadyCandidateInput {
            project_id,
            project_build_generation,
            origin_operation_id,
            origin,
            target,
            base_target_digest,
            source_lineage,
            contract_diff,
        } = input;
        let candidate_digest = digest_payload(&PluginReadyCandidateDigestInput {
            project_id: &project_id,
            project_build_generation,
            origin_operation_id: &origin_operation_id,
            origin,
            target: &target,
            base_target_digest: &base_target_digest,
            source_lineage: &source_lineage,
            contract_diff: &contract_diff,
        })?;
        let value = Self {
            candidate_id,
            candidate_digest,
            project_id,
            project_build_generation,
            origin_operation_id,
            origin,
            target,
            base_target_digest,
            source_lineage,
            contract_diff,
            matching_test_receipt: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.candidate_id.as_ref(), "candidate_id")?;
        validate_digest(&self.candidate_digest, "candidate_digest")?;
        validate_nonempty(
            self.origin_operation_id.as_ref(),
            "origin_operation_id",
        )?;
        self.target.validate()?;
        if let Some(base) = &self.base_target_digest {
            validate_digest(base, "base_target_digest")?;
        }
        self.source_lineage.validate()?;
        self.contract_diff.validate()?;
        if self.origin == PluginCandidateOrigin::Build && !self.source_lineage.is_managed() {
            return Err(PluginN1ContractError::InvalidField {
                field: "source_lineage",
                reason: "Build candidates require managed source lineage".into(),
            });
        }
        match (
            &self.source_lineage,
            self.project_build_generation,
        ) {
            (PluginSourceLineage::Managed { .. }, generation) if generation > 0 => {}
            (PluginSourceLineage::RuntimeOnly, _)
                if self.origin == PluginCandidateOrigin::Import => {}
            _ => {
                return Err(PluginN1ContractError::InvalidField {
                    field: "project_build_generation",
                    reason: "managed Source requires a positive Project generation; runtime-only candidates are import-only".into(),
                });
            }
        }
        validate_nonempty(self.project_id.as_ref(), "project_id")?;
        let expected = digest_payload(&PluginReadyCandidateDigestInput {
            project_id: &self.project_id,
            project_build_generation: self.project_build_generation,
            origin_operation_id: &self.origin_operation_id,
            origin: self.origin,
            target: &self.target,
            base_target_digest: &self.base_target_digest,
            source_lineage: &self.source_lineage,
            contract_diff: &self.contract_diff,
        })?;
        if expected != self.candidate_digest {
            return Err(PluginN1ContractError::DigestMismatch {
                field: "candidate_digest",
            });
        }
        if let Some(receipt) = &self.matching_test_receipt {
            receipt.validate()?;
            if receipt.candidate_id != self.candidate_id
                || receipt.candidate_digest != self.candidate_digest
            {
                return Err(PluginN1ContractError::InvalidField {
                    field: "matching_test_receipt",
                    reason: "receipt must bind the exact candidate ID and digest".into(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginManagedSourceHead {
    pub source_snapshot_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_lock_digest: Option<DigestHex>,
    pub build_profile_version: VersionString,
}

impl PluginManagedSourceHead {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_digest(
            &self.source_snapshot_digest,
            "managed_source.source_snapshot_digest",
        )?;
        if let Some(digest) = &self.dependency_lock_digest {
            validate_digest(digest, "managed_source.dependency_lock_digest")?;
        }
        require_version(
            &self.build_profile_version,
            PLUGIN_PACKAGE_PROFILE_VERSION,
            "managed_source.build_profile_version",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginProjectRecord {
    pub project_id: PluginProjectId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_mount_id: Option<PluginMountId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_source: Option<PluginManagedSourceHead>,
    pub build_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_candidate: Option<PluginReadyCandidateRef>,
    pub apply_mode: PluginApplyMode,
}

impl PluginProjectRecord {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.project_id.as_ref(), "project_id")?;
        if let Some(managed_source) = &self.managed_source {
            managed_source.validate()?;
        }
        if let Some(candidate) = &self.ready_candidate {
            candidate.validate()?;
        }
        if self.apply_mode == PluginApplyMode::AutoCompatibleWhenIdle
            && (self.linked_mount_id.is_none() || self.managed_source.is_none())
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "apply_mode",
                reason:
                    "auto Apply requires an exact linked Mount and managed Source"
                        .into(),
            });
        }
        Ok(())
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
pub enum CandidateTestOutcome {
    Passed,
    Failed,
    NeedsTestInput,
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
pub enum CandidateTestCredentialMode {
    None,
    OneShotCurrentBindings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateTestReceipt {
    pub receipt_id: CandidateTestReceiptId,
    pub candidate_id: PluginCandidateId,
    pub candidate_digest: DigestHex,
    pub outcome: CandidateTestOutcome,
    pub runtime: NodeRuntimeFingerprint,
    pub host_target: RuntimeTarget,
    pub host_contract_version: VersionString,
    pub javascript_sdk_contract_version: VersionString,
    pub test_contract_version: VersionString,
    pub source_lineage: PluginSourceLineage,
    pub credential_mode: CandidateTestCredentialMode,
    pub resolved_test_input_digest: DigestHex,
    pub host_generation: u64,
    pub issued_at_ms: i64,
}

impl CandidateTestReceipt {
    pub fn validate_for(
        &self,
        candidate: &PluginReadyCandidate,
    ) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "receipt_id")?;
        validate_digest(&self.candidate_digest, "candidate_digest")?;
        self.runtime.validate()?;
        validate_nonempty(self.host_target.as_ref(), "host_target")?;
        if self.host_target != self.runtime.runtime_target {
            return Err(PluginN1ContractError::InvalidField {
                field: "host_target",
                reason: "test target must equal the selected Runtime target".into(),
            });
        }
        require_version(
            &self.host_contract_version,
            JAVASCRIPT_HOST_PROTOCOL_VERSION,
            "host_contract_version",
        )?;
        require_version(
            &self.javascript_sdk_contract_version,
            JAVASCRIPT_SDK_CONTRACT_VERSION,
            "javascript_sdk_contract_version",
        )?;
        require_version(
            &self.test_contract_version,
            CANDIDATE_TEST_CONTRACT_VERSION,
            "test_contract_version",
        )?;
        self.source_lineage.validate()?;
        validate_digest(
            &self.resolved_test_input_digest,
            "resolved_test_input_digest",
        )?;
        if self.host_generation == 0 || self.issued_at_ms <= 0 {
            return Err(PluginN1ContractError::InvalidField {
                field: "test_host_identity",
                reason: "Host generation and receipt time must be positive".into(),
            });
        }
        if self.candidate_id != candidate.candidate_id
            || self.candidate_digest != candidate.candidate_digest
            || self.source_lineage != candidate.source_lineage
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "candidate",
                reason: "test receipt does not bind the exact candidate lineage".into(),
            });
        }
        Ok(())
    }

    pub fn reference(&self) -> CandidateTestReceiptRef {
        CandidateTestReceiptRef {
            receipt_id: self.receipt_id.clone(),
            candidate_id: self.candidate_id.clone(),
            candidate_digest: self.candidate_digest.clone(),
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
pub enum PluginApplyMode {
    AskBeforeApply,
    AutoCompatibleWhenIdle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginAutoApplyEligibility {
    pub standing_authorization_matches: bool,
    pub linked_mount_matches: bool,
    pub base_target_matches: bool,
    pub managed_source_matches_project_head: bool,
    pub matching_local_test_passed: bool,
    pub contribution_contracts_unchanged: bool,
    pub config_schema_unchanged: bool,
    pub credential_slots_unchanged: bool,
    pub resource_effect_contracts_unchanged: bool,
    pub runtime_requirement_unchanged: bool,
    pub dependency_lock_unchanged: bool,
    pub host_sdk_contract_unchanged: bool,
    pub supported_targets_unchanged: bool,
    pub static_validation_passed: bool,
    pub no_unknown_facts: bool,
}

impl PluginAutoApplyEligibility {
    pub fn is_eligible(&self) -> bool {
        self.standing_authorization_matches
            && self.linked_mount_matches
            && self.base_target_matches
            && self.managed_source_matches_project_head
            && self.matching_local_test_passed
            && self.contribution_contracts_unchanged
            && self.config_schema_unchanged
            && self.credential_slots_unchanged
            && self.resource_effect_contracts_unchanged
            && self.runtime_requirement_unchanged
            && self.dependency_lock_unchanged
            && self.host_sdk_contract_unchanged
            && self.supported_targets_unchanged
            && self.static_validation_passed
            && self.no_unknown_facts
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginApplyTarget {
    InitialInstall {
        allocated_mount_id: PluginMountId,
    },
    ExistingMount {
        mount_id: PluginMountId,
        expected_mount_revision: u64,
        expected_current_target_digest: DigestHex,
    },
}

impl PluginApplyTarget {
    fn mount_id(&self) -> &PluginMountId {
        match self {
            Self::InitialInstall { allocated_mount_id } => allocated_mount_id,
            Self::ExistingMount { mount_id, .. } => mount_id,
        }
    }

    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.mount_id().as_ref(), "apply_target.mount_id")?;
        if let Self::ExistingMount {
            expected_mount_revision,
            expected_current_target_digest,
            ..
        } = self
        {
            if *expected_mount_revision == 0 {
                return Err(PluginN1ContractError::InvalidField {
                    field: "expected_mount_revision",
                    reason: "existing Mount revision must be positive".into(),
                });
            }
            validate_digest(
                expected_current_target_digest,
                "expected_current_target_digest",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginApplyAuthorization {
    ManualUserConfirmation {
        confirmed_at_ms: i64,
        allow_breaking: bool,
    },
    StandingAuto {
        mount_id: PluginMountId,
        authorization_revision: u64,
    },
}

impl PluginApplyAuthorization {
    fn validate_for(
        &self,
        target: &PluginApplyTarget,
        compatibility: &PluginCompatibility,
    ) -> Result<(), PluginN1ContractError> {
        match self {
            Self::ManualUserConfirmation {
                confirmed_at_ms,
                allow_breaking,
            } => {
                if *confirmed_at_ms <= 0 {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "confirmed_at_ms",
                        reason: "manual confirmation time must be positive".into(),
                    });
                }
                if *compatibility == PluginCompatibility::Breaking
                    && !allow_breaking
                {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "allow_breaking",
                        reason: "breaking Apply requires explicit confirmation".into(),
                    });
                }
            }
            Self::StandingAuto {
                mount_id,
                authorization_revision,
            } => {
                if *authorization_revision == 0
                    || mount_id != target.mount_id()
                    || !matches!(target, PluginApplyTarget::ExistingMount { .. })
                    || *compatibility != PluginCompatibility::Compatible
                {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "standing_auto",
                        reason: "auto authorization must bind an existing compatible Mount and positive revision".into(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginHostCommitFence {
    NotResident,
    ResidentFenced {
        host_generation: u64,
        fence_token_digest: DigestHex,
    },
}

impl PluginHostCommitFence {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        if let Self::ResidentFenced {
            host_generation,
            fence_token_digest,
        } = self
        {
            if *host_generation == 0 {
                return Err(PluginN1ContractError::InvalidField {
                    field: "host_generation",
                    reason: "resident Host generation must be positive".into(),
                });
            }
            validate_digest(fence_token_digest, "fence_token_digest")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginApplyRequest {
    pub project_id: PluginProjectId,
    pub expected_project_build_generation: u64,
    pub candidate: PluginReadyCandidateRef,
    pub target: PluginApplyTarget,
    pub authorization: PluginApplyAuthorization,
    pub eligibility: PluginAutoApplyEligibility,
    pub commit_fence: PluginHostCommitFence,
}

impl PluginApplyRequest {
    pub fn validate_for(
        &self,
        candidate: &PluginReadyCandidate,
    ) -> Result<(), PluginN1ContractError> {
        self.candidate.validate()?;
        self.target.validate()?;
        self.commit_fence.validate()?;
        if self.candidate.candidate_id != candidate.candidate_id
            || self.candidate.candidate_digest != candidate.candidate_digest
            || self.project_id != candidate.project_id
            || self.expected_project_build_generation
                != candidate.project_build_generation
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "candidate",
                reason: "Apply request must bind the exact Candidate and Project generation"
                    .into(),
            });
        }
        match &self.target {
            PluginApplyTarget::InitialInstall { .. } => {
                if candidate.base_target_digest.is_some()
                    || matches!(
                        self.authorization,
                        PluginApplyAuthorization::StandingAuto { .. }
                    )
                {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "initial_install",
                        reason: "initial install has no base target and is always manual".into(),
                    });
                }
            }
            PluginApplyTarget::ExistingMount {
                expected_current_target_digest,
                ..
            } => {
                if candidate.base_target_digest.as_ref()
                    != Some(expected_current_target_digest)
                {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "base_target_digest",
                        reason: "Candidate base must equal the expected current target".into(),
                    });
                }
            }
        }
        self.authorization
            .validate_for(&self.target, &candidate.contract_diff.compatibility)?;
        if matches!(
            self.authorization,
            PluginApplyAuthorization::StandingAuto { .. }
        ) && !self.eligibility.is_eligible()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "eligibility",
                reason: "standing auto Apply requires every exact predicate".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginApplyResult {
    Applied {
        result: Box<PluginAppliedState>,
    },
    PreconditionsChanged {
        code: CanonicalErrorCode,
    },
    HostBusy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginAppliedState {
    pub mount_id: PluginMountId,
    pub mount_revision: u64,
    pub current_target: PluginTargetRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_target: Option<PluginTargetRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_host_generation: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRestorePreviousRequest {
    pub mount_id: PluginMountId,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: DigestHex,
    pub expected_previous_target_digest: DigestHex,
    pub commit_fence: PluginHostCommitFence,
    pub confirmed_at_ms: i64,
}

impl PluginRestorePreviousRequest {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.mount_id.as_ref(), "mount_id")?;
        if self.expected_mount_revision == 0 || self.confirmed_at_ms <= 0 {
            return Err(PluginN1ContractError::InvalidField {
                field: "restore_precondition",
                reason: "Mount revision and confirmation time must be positive".into(),
            });
        }
        validate_digest(
            &self.expected_current_target_digest,
            "expected_current_target_digest",
        )?;
        validate_digest(
            &self.expected_previous_target_digest,
            "expected_previous_target_digest",
        )?;
        self.commit_fence.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginRestorePreviousResult {
    Restored {
        result: Box<PluginRestoredState>,
    },
    PreconditionsChanged {
        code: CanonicalErrorCode,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRestoredState {
    pub mount_revision: u64,
    pub current_target: PluginTargetRef,
    pub previous_target: PluginTargetRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_host_generation: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateTestProvenance {
    pub outcome: CandidateTestOutcome,
    pub candidate_digest: DigestHex,
    pub runtime_target: RuntimeTarget,
    pub runtime_executable_digest: DigestHex,
    pub host_contract_version: VersionString,
    pub javascript_sdk_contract_version: VersionString,
    pub test_contract_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginShareSourceLineage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub originating_project_id: Option<PluginProjectId>,
    pub source_archive: LogicalArtifactRef,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_digest: DigestHex,
    pub build_profile_version: VersionString,
}

impl PluginShareSourceLineage {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_logical_artifact(&self.source_archive, "source_archive")?;
        validate_digest(
            &self.source_snapshot_digest,
            "source_snapshot_digest",
        )?;
        validate_digest(
            &self.dependency_lock_digest,
            "dependency_lock_digest",
        )?;
        require_version(
            &self.build_profile_version,
            PLUGIN_PACKAGE_PROFILE_VERSION,
            "build_profile_version",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginShareBundleManifest {
    pub schema_version: VersionString,
    pub bundle_format_version: VersionString,
    pub package_artifact: LogicalArtifactRef,
    pub package_artifact_digest: DigestHex,
    pub package_manifest_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_lineage: Option<PluginShareSourceLineage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_provenance: Option<CandidateTestProvenance>,
}

impl PluginShareBundleManifest {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        require_version(&self.schema_version, PLUGIN_N1_SCHEMA_VERSION, "schema_version")?;
        require_version(
            &self.bundle_format_version,
            PLUGIN_PACKAGE_PROFILE_VERSION,
            "bundle_format_version",
        )?;
        validate_logical_artifact(&self.package_artifact, "package_artifact")?;
        validate_digest(
            &self.package_artifact_digest,
            "package_artifact_digest",
        )?;
        validate_digest(
            &self.package_manifest_digest,
            "package_manifest_digest",
        )?;
        if let Some(source) = &self.source_lineage {
            source.validate()?;
        }
        if let Some(test) = &self.test_provenance {
            validate_digest(&test.candidate_digest, "test.candidate_digest")?;
            validate_nonempty(test.runtime_target.as_ref(), "test.runtime_target")?;
            validate_digest(
                &test.runtime_executable_digest,
                "test.runtime_executable_digest",
            )?;
            require_version(
                &test.host_contract_version,
                JAVASCRIPT_HOST_PROTOCOL_VERSION,
                "test.host_contract_version",
            )?;
            require_version(
                &test.javascript_sdk_contract_version,
                JAVASCRIPT_SDK_CONTRACT_VERSION,
                "test.javascript_sdk_contract_version",
            )?;
            require_version(
                &test.test_contract_version,
                CANDIDATE_TEST_CONTRACT_VERSION,
                "test.test_contract_version",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProductOperationOwner {
    PluginProject { project_id: PluginProjectId },
    PluginMount { mount_id: PluginMountId },
    Miniapp { miniapp_id: MiniAppId },
}

impl ProductOperationOwner {
    fn validate(&self) -> Result<(), PluginN1ContractError> {
        match self {
            Self::PluginProject { project_id } => {
                validate_nonempty(project_id.as_ref(), "project_id")
            }
            Self::PluginMount { mount_id } => {
                validate_nonempty(mount_id.as_ref(), "mount_id")
            }
            Self::Miniapp { miniapp_id } => {
                validate_nonempty(miniapp_id.as_ref(), "miniapp_id")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductOperationRecord {
    pub operation_id: OperationId,
    pub kind: ProductOperationKind,
    pub owner: ProductOperationOwner,
    pub state: ProductOperationState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<CanonicalErrorCode>,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
}

impl ProductOperationRecord {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        validate_nonempty(self.operation_id.as_ref(), "operation_id")?;
        self.owner.validate()?;
        let owner_allowed = match self.kind {
            ProductOperationKind::Build => matches!(
                self.owner,
                ProductOperationOwner::PluginProject { .. }
                    | ProductOperationOwner::Miniapp { .. }
            ),
            ProductOperationKind::Import | ProductOperationKind::Export => true,
            ProductOperationKind::MiniappPermanentDelete => {
                matches!(self.owner, ProductOperationOwner::Miniapp { .. })
            }
        };
        if !owner_allowed {
            return Err(PluginN1ContractError::InvalidField {
                field: "owner",
                reason: "operation kind is not valid for this owner".into(),
            });
        }
        if self.started_at_ms <= 0
            || self
                .progress_percent
                .is_some_and(|progress| progress > 100)
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "operation_progress",
                reason: "progress must be 0..=100 and started_at_ms positive".into(),
            });
        }
        if self.kind == ProductOperationKind::MiniappPermanentDelete
            && self.progress_percent.is_some()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "progress_percent",
                reason: "MiniApp permanent delete does not persist phase progress".into(),
            });
        }
        if self.state.is_terminal() != self.finished_at_ms.is_some() {
            return Err(PluginN1ContractError::InvalidField {
                field: "finished_at_ms",
                reason: "terminal state and completion time must agree".into(),
            });
        }
        if self
            .finished_at_ms
            .is_some_and(|finished| finished < self.started_at_ms)
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "finished_at_ms",
                reason: "completion time cannot precede start time".into(),
            });
        }
        if (self.state == ProductOperationState::Failed) != self.last_error.is_some() {
            return Err(PluginN1ContractError::InvalidField {
                field: "last_error",
                reason: "only failed operations carry a terminal error".into(),
            });
        }
        if self.state == ProductOperationState::Succeeded
            && self.kind != ProductOperationKind::MiniappPermanentDelete
            && self.progress_percent != Some(100)
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "progress_percent",
                reason: "successful progress-reporting operations finish at 100".into(),
            });
        }
        if self.kind == ProductOperationKind::MiniappPermanentDelete
            && self.state == ProductOperationState::Canceled
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "state",
                reason: "MiniApp permanent delete is not cancelable after admission".into(),
            });
        }
        if self.bounded_log_tail.len() > 200 {
            return Err(PluginN1ContractError::InvalidField {
                field: "bounded_log_tail",
                reason: "operation log tail exceeds 200 lines".into(),
            });
        }
        Ok(())
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
pub enum PlatformValidationStage {
    Candidate,
    SignedRc,
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
pub enum PlatformCellRequirement {
    Required,
    Optional,
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
pub enum PlatformValidationStatus {
    Pass,
    Fail,
    NotDelivered,
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
pub enum PlatformNotDeliveredReason {
    NotInReleaseScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct N1PlatformValidationRecord {
    pub schema_version: VersionString,
    pub cohort_id: ValidationCohortId,
    pub source_commit: String,
    pub stage: PlatformValidationStage,
    pub cell_id: N1PlatformCellId,
    pub requirement: PlatformCellRequirement,
    pub host_target: RuntimeTarget,
    pub status: PlatformValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_delivered_reason: Option<PlatformNotDeliveredReason>,
    pub artifact_digests: BTreeMap<String, DigestHex>,
    pub check_ids: BTreeSet<String>,
}

impl N1PlatformValidationRecord {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        require_version(&self.schema_version, PLUGIN_N1_SCHEMA_VERSION, "schema_version")?;
        validate_nonempty(self.cohort_id.as_ref(), "cohort_id")?;
        validate_git_commit(&self.source_commit)?;
        validate_nonempty(self.host_target.as_ref(), "host_target")?;
        let expected_requirement = if N1PlatformCellId::REQUIRED.contains(&self.cell_id) {
            PlatformCellRequirement::Required
        } else {
            PlatformCellRequirement::Optional
        };
        if self.requirement != expected_requirement {
            return Err(PluginN1ContractError::InvalidField {
                field: "requirement",
                reason: "cell requirement differs from the frozen release matrix".into(),
            });
        }
        if self.requirement == PlatformCellRequirement::Required
            && self.status == PlatformValidationStatus::NotDelivered
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "status",
                reason: "required cells cannot be marked not_delivered".into(),
            });
        }
        if (self.status == PlatformValidationStatus::NotDelivered)
            != self.not_delivered_reason.is_some()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "not_delivered_reason",
                reason: "only not-delivered optional cells carry a release-scope reason".into(),
            });
        }
        if self.status == PlatformValidationStatus::Pass
            && self.artifact_digests.is_empty()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "evidence",
                reason: "passing platform record requires artifact digests".into(),
            });
        }
        if self.status != PlatformValidationStatus::NotDelivered
            && self.check_ids.is_empty()
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "check_ids",
                reason: "executed platform records require at least one check".into(),
            });
        }
        for (name, digest) in &self.artifact_digests {
            validate_machine_key(name, "artifact_digests.key")?;
            validate_digest(digest, "artifact_digests.value")?;
        }
        for check in &self.check_ids {
            validate_machine_key(check, "check_ids")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct N1CohortCellLock {
    pub requirement: PlatformCellRequirement,
    pub validation_manifest_digests:
        BTreeMap<PlatformValidationStage, DigestHex>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct N1CohortLock {
    pub schema_version: VersionString,
    pub cohort_id: ValidationCohortId,
    pub source_commit: String,
    pub input_digests: BTreeMap<String, DigestHex>,
    pub cells: BTreeMap<N1PlatformCellId, N1CohortCellLock>,
}

impl N1CohortLock {
    pub fn validate(&self) -> Result<(), PluginN1ContractError> {
        require_version(&self.schema_version, PLUGIN_N1_SCHEMA_VERSION, "schema_version")?;
        validate_nonempty(self.cohort_id.as_ref(), "cohort_id")?;
        validate_git_commit(&self.source_commit)?;
        if self.input_digests.is_empty() {
            return Err(PluginN1ContractError::InvalidField {
                field: "input_digests",
                reason: "cohort lock requires frozen input digests".into(),
            });
        }
        for (name, digest) in &self.input_digests {
            validate_machine_key(name, "input_digests.key")?;
            validate_digest(digest, "input_digests.value")?;
        }
        for (cell_id, cell) in &self.cells {
            let expected = if N1PlatformCellId::REQUIRED.contains(cell_id) {
                PlatformCellRequirement::Required
            } else {
                PlatformCellRequirement::Optional
            };
            if cell.requirement != expected {
                return Err(PluginN1ContractError::InvalidField {
                    field: "cells.requirement",
                    reason: format!("cell {cell_id:?} requirement mismatch"),
                });
            }
            if cell.validation_manifest_digests.is_empty() {
                return Err(PluginN1ContractError::InvalidField {
                    field: "cells.validation_manifest_digests",
                    reason: "a delivered cell must reference at least one validation record"
                        .into(),
                });
            }
            for digest in cell.validation_manifest_digests.values() {
                validate_digest(digest, "cells.validation_manifest_digests")?;
            }
        }
        Ok(())
    }

    pub fn validate_stable_promotion(&self) -> Result<(), PluginN1ContractError> {
        self.validate()?;
        for required in N1PlatformCellId::REQUIRED {
            let cell = self.cells.get(&required).ok_or_else(|| {
                PluginN1ContractError::InvalidField {
                    field: "cells",
                    reason: format!(
                        "required cell {required:?} is missing from Stable promotion"
                    ),
                }
            })?;
            for stage in [
                PlatformValidationStage::Candidate,
                PlatformValidationStage::SignedRc,
            ] {
                if !cell.validation_manifest_digests.contains_key(&stage) {
                    return Err(PluginN1ContractError::InvalidField {
                        field: "cells.validation_manifest_digests",
                        reason: format!(
                            "required cell {required:?} is missing {stage:?} evidence"
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum PluginN1ContractError {
    #[error("{field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
    #[error("duplicate {field}: {value}")]
    DuplicateIdentity {
        field: &'static str,
        value: String,
    },
    #[error("{field} digest mismatch")]
    DigestMismatch { field: &'static str },
    #[error(transparent)]
    Digest(#[from] crate::CanonicalDigestError),
}

fn require_version(
    value: &VersionString,
    expected: &str,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    if value.as_ref() == expected {
        Ok(())
    } else {
        Err(PluginN1ContractError::InvalidField {
            field,
            reason: format!("expected {expected}, observed {}", value.as_ref()),
        })
    }
}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), PluginN1ContractError> {
    if value.is_empty() || value.trim() != value {
        return Err(PluginN1ContractError::InvalidField {
            field,
            reason: "value must be non-empty and trimmed".into(),
        });
    }
    Ok(())
}

fn validate_absolute_path(
    value: &str,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    validate_nonempty(value, field)?;
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let windows_unc = value.starts_with("\\\\");
    let unix_root = value.starts_with('/');
    if windows_drive || windows_unc || unix_root {
        Ok(())
    } else {
        Err(PluginN1ContractError::InvalidField {
            field,
            reason: "path must be absolute".into(),
        })
    }
}

fn validate_host_request(
    request: &PluginHostRequest,
) -> Result<(), PluginN1ContractError> {
    match request {
        PluginHostRequest::MountLoad { context } => context.validate(),
        PluginHostRequest::MountUnload { target } => target.validate(),
        PluginHostRequest::CapabilityInvoke {
            contribution,
            action_id,
            ..
        } => {
            contribution.validate()?;
            validate_nonempty(action_id.as_ref(), "action_id")
        }
        PluginHostRequest::ContextContribute {
            contribution,
            schema_ref,
        } => {
            contribution.validate()?;
            validate_nonempty(schema_ref.as_ref(), "schema_ref")
        }
        PluginHostRequest::ResourceAcquire {
            contribution,
            binding_id,
            resource_kind,
            parameters,
        } => {
            contribution.validate()?;
            validate_nonempty(binding_id.as_ref(), "binding_id")?;
            validate_nonempty(resource_kind.as_ref(), "resource_kind")?;
            if !parameters.0.is_object() {
                return Err(PluginN1ContractError::InvalidField {
                    field: "parameters",
                    reason: "resource parameters must be a JSON object".into(),
                });
            }
            Ok(())
        }
        PluginHostRequest::ResourceRelease { handle_id } => {
            validate_nonempty(handle_id, "handle_id")
        }
        PluginHostRequest::CredentialResolve {
            mount_handle_id,
            slot_key,
        } => {
            validate_nonempty(mount_handle_id, "mount_handle_id")?;
            validate_machine_key(slot_key.as_ref(), "slot_key")
        }
        PluginHostRequest::StateGet {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateSet {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateDelete {
            mount_handle_id, ..
        }
        | PluginHostRequest::StateCompareAndSwap {
            mount_handle_id, ..
        } => {
            validate_nonempty(mount_handle_id, "mount_handle_id")
        }
        PluginHostRequest::BuildExecute {
            source_root,
            staging_root,
            source_snapshot_digest,
            dependency_lock_digest,
            ..
        } => {
            validate_absolute_path(source_root, "source_root")?;
            validate_absolute_path(staging_root, "staging_root")?;
            validate_digest(source_snapshot_digest, "source_snapshot_digest")?;
            validate_digest(dependency_lock_digest, "dependency_lock_digest")
        }
        PluginHostRequest::RequestCancel { target_request_id } => {
            validate_nonempty(target_request_id.as_ref(), "target_request_id")
        }
        PluginHostRequest::HostShutdown => Ok(()),
    }
}

fn validate_host_response(
    request: &PluginHostRequestEnvelope,
    response: &PluginHostResponseBody,
) -> Result<(), PluginN1ContractError> {
    let PluginHostResponseBody::Success(success) = response else {
        let PluginHostResponseBody::Failure(error) = response else {
            unreachable!()
        };
        validate_nonempty(error.code.as_ref(), "error.code")?;
        return validate_nonempty(&error.message, "error.message");
    };

    let valid_shape = matches!(
        (&request.request, success),
        (
            PluginHostRequest::MountLoad { .. }
                | PluginHostRequest::MountUnload { .. }
                | PluginHostRequest::ResourceRelease { .. }
                | PluginHostRequest::RequestCancel { .. }
                | PluginHostRequest::HostShutdown,
            PluginHostSuccess::Ack
        ) | (
            PluginHostRequest::CapabilityInvoke { .. }
                | PluginHostRequest::ContextContribute { .. },
            PluginHostSuccess::Value(_)
        ) | (
            PluginHostRequest::ResourceAcquire { .. },
            PluginHostSuccess::ResourceAcquired { .. }
        ) | (
            PluginHostRequest::CredentialResolve { .. },
            PluginHostSuccess::CredentialResolved { .. }
        ) | (
            PluginHostRequest::StateGet { .. },
            PluginHostSuccess::StateGet(_)
        ) | (
            PluginHostRequest::StateSet { .. },
            PluginHostSuccess::StateSet(_)
        ) | (
            PluginHostRequest::StateDelete { .. },
            PluginHostSuccess::StateDelete(_)
        ) | (
            PluginHostRequest::StateCompareAndSwap { .. },
            PluginHostSuccess::StateCompareAndSwap(_)
        ) | (
            PluginHostRequest::BuildExecute { .. },
            PluginHostSuccess::BuildCompleted { .. }
        )
    );
    if !valid_shape {
        return Err(PluginN1ContractError::InvalidField {
            field: "response",
            reason: "success payload does not match the request method".into(),
        });
    }
    match (&request.request, success) {
        (
            PluginHostRequest::CredentialResolve { slot_key, .. },
            PluginHostSuccess::CredentialResolved {
                slot_key: observed,
                secret,
            },
        ) => {
            if slot_key != observed {
                return Err(PluginN1ContractError::InvalidField {
                    field: "response.slot_key",
                    reason: "Credential response must bind the requested slot".into(),
                });
            }
            validate_nonempty(&secret.0, "response.secret")
        }
        (
            PluginHostRequest::ResourceAcquire { .. },
            PluginHostSuccess::ResourceAcquired { handle_id },
        ) => validate_nonempty(handle_id, "response.handle_id"),
        (
            PluginHostRequest::BuildExecute { .. },
            PluginHostSuccess::BuildCompleted {
                artifact_id,
                artifact_digest,
                manifest_digest,
            },
        ) => {
            validate_nonempty(artifact_id.as_ref(), "response.artifact_id")?;
            validate_digest(artifact_digest, "response.artifact_digest")?;
            validate_digest(manifest_digest, "response.manifest_digest")
        }
        _ if valid_shape => Ok(()),
        _ => Err(PluginN1ContractError::InvalidField {
            field: "response",
            reason: "success payload does not match the request method".into(),
        }),
    }
}

fn validate_package_contributions(
    package: &PackageRef,
    contributions: &PackageContributions,
) -> Result<(), PluginN1ContractError> {
    if !contributions.role_contracts.is_empty()
        || !contributions.role_providers.is_empty()
    {
        return Err(PluginN1ContractError::InvalidField {
            field: "contributions",
            reason: "Plugin Package v1 cannot publish Role contracts or Role providers"
                .into(),
        });
    }
    if contributions.capabilities.is_empty()
        && contributions.skills.is_empty()
        && contributions.mcp_tools.is_empty()
    {
        return Err(PluginN1ContractError::InvalidField {
            field: "contributions",
            reason: "Plugin Package v1 must publish at least one contribution".into(),
        });
    }

    let mut capability_ids = BTreeSet::new();
    let mut contribution_ids = BTreeSet::new();
    for capability in &contributions.capabilities {
        validate_nonempty(capability.id.as_ref(), "capability.id")?;
        validate_nonempty(
            capability.contribution_id.as_ref(),
            "capability.contribution_id",
        )?;
        validate_nonempty(capability.version.as_ref(), "capability.version")?;
        if capability.package != *package {
            return Err(PluginN1ContractError::InvalidField {
                field: "capability.package",
                reason: "Capability must be owned by the enclosing Package".into(),
            });
        }
        if !capability_ids.insert(capability.id.clone()) {
            return Err(PluginN1ContractError::DuplicateIdentity {
                field: "capability.id",
                value: capability.id.as_ref().to_owned(),
            });
        }
        if !contribution_ids.insert(capability.contribution_id.clone()) {
            return Err(PluginN1ContractError::DuplicateIdentity {
                field: "capability.contribution_id",
                value: capability.contribution_id.as_ref().to_owned(),
            });
        }
        if !matches!(
            capability.kind,
            crate::CapabilityKind::Tool
                | crate::CapabilityKind::ContextContributor
                | crate::CapabilityKind::ResourceProvider
        ) {
            return Err(PluginN1ContractError::InvalidField {
                field: "capability.kind",
                reason: "Plugin Package v1 supports only Tool, Context Contributor, and Resource Provider capabilities".into(),
            });
        }
        let supported_consumers = capability
            .supported_consumers()
            .map_err(|reason| PluginN1ContractError::InvalidField {
                field: "capability.supported_surfaces",
                reason,
            })?;
        if supported_consumers.is_empty() || capability.supported_platforms.is_empty() {
            return Err(PluginN1ContractError::InvalidField {
                field: "capability.availability",
                reason:
                    "Capability must declare at least one consumer and platform constraint"
                        .into(),
            });
        }
        if !capability.config_schema.0.is_object() {
            return Err(PluginN1ContractError::InvalidField {
                field: "capability.config_schema",
                reason: "Capability config schema must be a JSON object".into(),
            });
        }

        let mut action_ids = BTreeSet::new();
        for action in &capability.contributions.actions {
            validate_nonempty(action.action_id.as_ref(), "capability.action_id")?;
            validate_nonempty(
                action.input_schema.as_ref(),
                "capability.action.input_schema",
            )?;
            validate_nonempty(
                action.output_schema.as_ref(),
                "capability.action.output_schema",
            )?;
            if !action_ids.insert(action.action_id.clone()) {
                return Err(PluginN1ContractError::DuplicateIdentity {
                    field: "capability.action_id",
                    value: action.action_id.as_ref().to_owned(),
                });
            }
        }
        match capability.kind {
            crate::CapabilityKind::Tool
                if capability.contributions.actions.is_empty() =>
            {
                return Err(PluginN1ContractError::InvalidField {
                    field: "capability.actions",
                    reason: "Tool capability requires at least one action".into(),
                });
            }
            crate::CapabilityKind::ContextContributor
                if capability.contributions.context_schema_refs.is_empty() =>
            {
                return Err(PluginN1ContractError::InvalidField {
                    field: "capability.context_schema_refs",
                    reason: "Context Contributor requires at least one schema".into(),
                });
            }
            crate::CapabilityKind::ResourceProvider
                if capability.contributions.resource_kinds.is_empty() =>
            {
                return Err(PluginN1ContractError::InvalidField {
                    field: "capability.resource_kinds",
                    reason: "Resource Provider requires at least one resource kind".into(),
                });
            }
            _ => {}
        }
    }

    let mut skill_ids = BTreeSet::new();
    for skill in &contributions.skills {
        validate_nonempty(skill.id.as_ref(), "skill.id")?;
        validate_nonempty(skill.version.as_ref(), "skill.version")?;
        if skill.package != *package {
            return Err(PluginN1ContractError::InvalidField {
                field: "skill.package",
                reason: "Skill must be owned by the enclosing Package".into(),
            });
        }
        if !skill_ids.insert(skill.id.clone()) {
            return Err(PluginN1ContractError::DuplicateIdentity {
                field: "skill.id",
                value: skill.id.as_ref().to_owned(),
            });
        }
        validate_logical_artifact(&skill.body_ref, "skill.body_ref")?;
        for resource in &skill.resources {
            validate_logical_artifact(&resource.artifact, "skill.resource")?;
        }
        if !skill
            .supported_surfaces
            .iter()
            .any(|surface| surface.starts_with(crate::CAPABILITY_CONSUMER_SURFACE_PREFIX))
        {
            return Err(PluginN1ContractError::InvalidField {
                field: "skill.supported_surfaces",
                reason: "Skill must declare at least one consumer:<id> surface".into(),
            });
        }
    }

    let mut mcp_keys = BTreeSet::new();
    for mapping in &contributions.mcp_tools {
        if mapping.package != *package {
            return Err(PluginN1ContractError::InvalidField {
                field: "mcp.package",
                reason: "MCP mapping must be owned by the enclosing Package".into(),
            });
        }
        validate_nonempty(mapping.server_id.as_ref(), "mcp.server_id")?;
        validate_nonempty(
            mapping.canonical_tool_key.as_ref(),
            "mcp.canonical_tool_key",
        )?;
        validate_digest(&mapping.schema_digest, "mcp.schema_digest")?;
        if !mcp_keys.insert((
            mapping.server_id.clone(),
            mapping.canonical_tool_key.clone(),
        )) {
            return Err(PluginN1ContractError::DuplicateIdentity {
                field: "mcp.mapping",
                value: mapping.canonical_tool_key.as_ref().to_owned(),
            });
        }
        if !contributions.capabilities.iter().any(|capability| {
            capability.id == mapping.capability.id
                && capability.version == mapping.capability.version
                && capability.kind == crate::CapabilityKind::Tool
        }) {
            return Err(PluginN1ContractError::InvalidField {
                field: "mcp.capability",
                reason: "MCP mapping must reference an exact Tool capability in this Package"
                    .into(),
            });
        }
    }
    Ok(())
}

fn validate_machine_key(
    value: &str,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    validate_nonempty(value, field)?;
    let mut chars = value.chars();
    if !chars
        .next()
        .is_some_and(|value| value.is_ascii_lowercase())
        || !chars.all(|value| {
            value.is_ascii_lowercase()
                || value.is_ascii_digit()
                || matches!(value, '.' | '_' | '-')
        })
    {
        return Err(PluginN1ContractError::InvalidField {
            field,
            reason: "machine key must match [a-z][a-z0-9._-]*".into(),
        });
    }
    Ok(())
}

fn validate_digest(
    value: &DigestHex,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    if value.as_ref().len() != 64
        || !value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PluginN1ContractError::InvalidField {
            field,
            reason: "digest must be 64 lowercase hexadecimal characters".into(),
        });
    }
    Ok(())
}

fn validate_relative_path(
    value: &str,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    validate_nonempty(value, field)?;
    if value.starts_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(PluginN1ContractError::InvalidField {
            field,
            reason: "path must be normalized, relative, and traversal-free".into(),
        });
    }
    Ok(())
}

fn validate_logical_artifact(
    value: &LogicalArtifactRef,
    field: &'static str,
) -> Result<(), PluginN1ContractError> {
    validate_nonempty(value.artifact_id.as_ref(), field)?;
    validate_relative_path(&value.normalized_relative_path, field)?;
    validate_digest(&value.digest, field)
}

fn validate_git_commit(value: &str) -> Result<(), PluginN1ContractError> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PluginN1ContractError::InvalidField {
            field: "source_commit",
            reason: "source commit must be a 40-character lowercase Git SHA".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, ExactVersionRef, JavaScriptEntrypointMetadata, LocalizedMetadata,
        PackageContributions, PackageId, PlatformConstraint,
    };

    fn digest(value: char) -> DigestHex {
        DigestHex::from(value.to_string().repeat(64))
    }

    fn runtime() -> NodeRuntimeFingerprint {
        NodeRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("node-managed-24"),
            source_kind: NodeRuntimeSourceKind::Managed,
            node_version: VersionString::from("24.8.0"),
            node_major: RECOMMENDED_NODE_LTS_MAJOR,
            runtime_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            executable_digest: digest('a'),
            javascript_host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        }
    }

    fn manifest() -> PluginPackageV1Manifest {
        let package = ExactVersionRef {
            id: PackageId::from("example.csv"),
            version: VersionString::from("1.0.0"),
        };
        let capability = CapabilityManifest {
            id: "example.csv.read".into(),
            contribution_id: ContributionId::from("capability:example.csv.read"),
            version: "1.0.0".into(),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "CSV Read".into(),
                description: "Read a CSV resource.".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions {
                actions: vec![crate::CapabilityActionDescriptor {
                    action_id: ActionId::from("example.csv.read.invoke"),
                    input_schema: CanonicalSchemaRef::from(
                        "schema://example.csv/read-input@1",
                    ),
                    output_schema: CanonicalSchemaRef::from(
                        "schema://example.csv/read-output@1",
                    ),
                    effect_class: crate::EffectClass::ReadLocal,
                    presentation: crate::ToolPresentationKind::FunctionTool,
                }],
                ..Default::default()
            },
            supported_surfaces: crate::capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
            ),
        };
        PluginPackageV1Manifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            build_profile: JavaScriptBuildProfile::PluginPackageV1,
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package: PackageManifest {
                schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                package_id: package.id.clone(),
                package_version: package.version.clone(),
                display: LocalizedMetadata {
                    name: "CSV Plugin".into(),
                    description: "CSV tools.".into(),
                    localized_names: BTreeMap::new(),
                    localized_descriptions: BTreeMap::new(),
                },
                package_dependencies: Vec::new(),
                requires_runtime_features: Vec::new(),
                config_schema: StrictJsonValue(json!({"type": "object"})),
                provides_services: Vec::new(),
                requires_services: Vec::new(),
                entrypoint: JavaScriptEntrypointMetadata {
                    normalized_relative_path: "main.mjs".into(),
                    module_digest: digest('c'),
                    host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                    sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                }
                .into(),
                contributions: PackageContributions {
                    capabilities: vec![capability],
                    ..Default::default()
                },
            },
            supported_targets: BTreeSet::from([RuntimeTarget::from(
                "x86_64-pc-windows-msvc",
            )]),
            minimum_node_major: MINIMUM_NODE_MAJOR,
            dependency_lock_digest: digest('b'),
            credential_slots: vec![CredentialSlotDeclaration {
                slot_key: CredentialSlotKey::from("api_key"),
                kind: CredentialSlotKind::SecretText,
                display_name: "API key".into(),
                required: false,
            }],
        }
    }

    fn candidate() -> PluginReadyCandidate {
        PluginReadyCandidate::new(
            PluginCandidateId::from("candidate-1"),
            PluginReadyCandidateInput {
                project_id: PluginProjectId::from("project-1"),
                project_build_generation: 1,
                origin_operation_id: OperationId::from("build-1"),
                origin: PluginCandidateOrigin::Build,
                target: PluginTargetRef {
                    package: manifest().package_ref(),
                    artifact_id: ArtifactId::from("artifact-1"),
                    artifact_digest: digest('d'),
                    manifest_digest: digest('e'),
                },
                base_target_digest: Some(digest('f')),
                source_lineage: PluginSourceLineage::Managed {
                    source_snapshot_digest: digest('1'),
                    dependency_lock_digest: digest('2'),
                    build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
                },
                contract_diff: PluginContractDiff {
                    compatibility: PluginCompatibility::Compatible,
                    changes: BTreeSet::from([PluginContractChangeKind::ArtifactBytes]),
                    affected_consumer_locks: Vec::new(),
                },
            },
        )
        .unwrap()
    }

    fn auto_eligibility() -> PluginAutoApplyEligibility {
        PluginAutoApplyEligibility {
            standing_authorization_matches: true,
            linked_mount_matches: true,
            base_target_matches: true,
            managed_source_matches_project_head: true,
            matching_local_test_passed: true,
            contribution_contracts_unchanged: true,
            config_schema_unchanged: true,
            credential_slots_unchanged: true,
            resource_effect_contracts_unchanged: true,
            runtime_requirement_unchanged: true,
            dependency_lock_unchanged: true,
            host_sdk_contract_unchanged: true,
            supported_targets_unchanged: true,
            static_validation_passed: true,
            no_unknown_facts: true,
        }
    }

    #[test]
    fn canonical_contract_freezes_exact_host_operation_and_platform_sets() {
        let contract = PluginN1ContractManifest::canonical();
        contract.validate().unwrap();
        assert_eq!(contract.minimum_node_major, 22);
        assert_eq!(contract.recommended_node_lts_major, 24);
        assert_eq!(contract.host_method_sets.len(), 3);
        assert_eq!(
            contract.host_method_sets[&JavaScriptHostKind::Build].len(),
            3
        );
        assert!(!contract.host_method_sets[&JavaScriptHostKind::Build]
            .contains(&JavaScriptHostMethod::CapabilityInvoke));
        assert_eq!(contract.durable_operation_kinds.len(), 4);
        assert_eq!(contract.required_cells.len(), 3);
        assert_eq!(contract.optional_cells.len(), 2);
    }

    #[test]
    fn package_artifact_binds_main_module_and_tree_digest() {
        let artifact = PluginPackageArtifactV1::new(
            ArtifactId::from("artifact-1"),
            manifest(),
            vec![
                ArtifactFileDigest {
                    normalized_relative_path: "resources/example.csv".into(),
                    digest: digest('d'),
                    size_bytes: 4,
                },
                ArtifactFileDigest {
                    normalized_relative_path: "main.mjs".into(),
                    digest: digest('c'),
                    size_bytes: 12,
                },
            ],
        )
        .unwrap();
        artifact.validate().unwrap();

        let mut tampered = artifact;
        tampered.files[0].digest = digest('e');
        assert!(matches!(
            tampered.validate(),
            Err(PluginN1ContractError::DigestMismatch {
                field: "entrypoint.digest"
            })
        ));
    }

    #[test]
    fn host_role_and_generation_are_exact_wire_fences() {
        let contract = PluginN1ContractManifest::canonical();
        let request = PluginHostRequestEnvelope {
            protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            host_kind: JavaScriptHostKind::Build,
            host_generation: 7,
            request_id: CorrelationId::from("request-1"),
            direction: JavaScriptHostMessageDirection::HostToJavaScript,
            request: PluginHostRequest::CapabilityInvoke {
                contribution: PluginHostContributionRef {
                    target: PluginHostTargetLock {
                        mount_id: PluginMountId::from("mount-1"),
                        package: manifest().package_ref(),
                        artifact_digest: digest('a'),
                        manifest_digest: digest('b'),
                    },
                    contribution_id: ContributionId::from(
                        "capability:example.csv.read",
                    ),
                    capability: CapabilityRef {
                        id: CapabilityId::from("example.csv.read"),
                        version: VersionString::from("1.0.0"),
                    },
                    contract_digest: digest('c'),
                },
                action_id: ActionId::from("example.csv.read.invoke"),
                input: StrictJsonValue(json!({})),
            },
        };
        assert!(request.validate(&contract).is_err());

        let valid_request = PluginHostRequestEnvelope {
            request: PluginHostRequest::BuildExecute {
                profile: JavaScriptBuildProfile::PluginPackageV1,
                source_root: "C:\\source".into(),
                staging_root: "C:\\staging".into(),
                source_snapshot_digest: digest('d'),
                dependency_lock_digest: digest('e'),
            },
            ..request
        };
        valid_request.validate(&contract).unwrap();

        let stale = PluginHostResponseEnvelope {
            protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            host_kind: JavaScriptHostKind::Build,
            host_generation: 6,
            request_id: valid_request.request_id.clone(),
            response: PluginHostResponseBody::Success(
                PluginHostSuccess::Ack,
            ),
        };
        assert!(stale.validate_for(&valid_request).is_err());
    }

    #[test]
    fn runtime_selection_binds_validation_to_pending_candidate() {
        let pending = runtime();
        let record = RuntimeSelectionRecord {
            selected_runtime: None,
            pending_candidate: Some(pending.clone()),
            validation_result: Some(RuntimeSwitchValidationResult {
                candidate: pending,
                foundation_hello_passed: true,
                old_runtime_process_tree_zero: true,
                participants: vec![RuntimeSwitchParticipantResult {
                    kind: RuntimeSwitchParticipantKind::BuildFoundation,
                    owner_id: "javascript-build-foundation".into(),
                    outcome: RuntimeSwitchParticipantOutcome::Passed,
                    error_code: None,
                }],
                completed_at_ms: 1,
            }),
            last_error: None,
            non_recommended_warning_acknowledged: BTreeSet::new(),
        };
        record.validate().unwrap();

        let mut stale = record;
        stale.pending_candidate.as_mut().unwrap().executable_digest = digest('f');
        assert!(stale.validate().is_err());
    }

    #[test]
    fn runtime_only_import_cannot_masquerade_as_a_build_candidate() {
        let mut value = candidate();
        value.origin = PluginCandidateOrigin::Build;
        value.source_lineage = PluginSourceLineage::RuntimeOnly;
        assert!(value.validate().is_err());
    }

    #[test]
    fn test_receipt_binds_exact_candidate_runtime_and_source_lineage() {
        let candidate = candidate();
        let receipt = CandidateTestReceipt {
            receipt_id: CandidateTestReceiptId::from("receipt-1"),
            candidate_id: candidate.candidate_id.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
            outcome: CandidateTestOutcome::Passed,
            runtime: runtime(),
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            host_contract_version: VersionString::from("1.0.0"),
            javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            test_contract_version: CANDIDATE_TEST_CONTRACT_VERSION.into(),
            source_lineage: candidate.source_lineage.clone(),
            credential_mode: CandidateTestCredentialMode::None,
            resolved_test_input_digest: digest('3'),
            host_generation: 1,
            issued_at_ms: 1,
        };
        receipt.validate_for(&candidate).unwrap();
        assert_eq!(receipt.reference().candidate_id, candidate.candidate_id);
    }

    #[test]
    fn auto_apply_requires_every_exact_fact_and_user_authorization() {
        let mut facts = auto_eligibility();
        assert!(facts.is_eligible());
        facts.standing_authorization_matches = false;
        assert!(!facts.is_eligible());
    }

    #[test]
    fn auto_apply_binds_candidate_base_generation_and_authorization() {
        let candidate = candidate();
        let request = PluginApplyRequest {
            project_id: candidate.project_id.clone(),
            expected_project_build_generation: candidate.project_build_generation,
            candidate: PluginReadyCandidateRef {
                candidate_id: candidate.candidate_id.clone(),
                candidate_digest: candidate.candidate_digest.clone(),
            },
            target: PluginApplyTarget::ExistingMount {
                mount_id: PluginMountId::from("mount-1"),
                expected_mount_revision: 2,
                expected_current_target_digest: candidate
                    .base_target_digest
                    .clone()
                    .unwrap(),
            },
            authorization: PluginApplyAuthorization::StandingAuto {
                mount_id: PluginMountId::from("mount-1"),
                authorization_revision: 3,
            },
            eligibility: auto_eligibility(),
            commit_fence: PluginHostCommitFence::ResidentFenced {
                host_generation: 4,
                fence_token_digest: digest('4'),
            },
        };
        request.validate_for(&candidate).unwrap();

        let mut stale = request;
        stale.expected_project_build_generation = 9;
        assert!(stale.validate_for(&candidate).is_err());
    }

    #[test]
    fn share_bundle_source_lineage_is_complete_or_absent() {
        let mut bundle = PluginShareBundleManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            bundle_format_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package_artifact: LogicalArtifactRef {
                artifact_id: ArtifactId::from("artifact-1"),
                normalized_relative_path: "artifact/plugin.nfpkg".into(),
                digest: digest('a'),
            },
            package_artifact_digest: digest('a'),
            package_manifest_digest: digest('b'),
            source_lineage: None,
            test_provenance: None,
        };
        bundle.validate().unwrap();
        bundle.source_lineage = Some(PluginShareSourceLineage {
            originating_project_id: Some(PluginProjectId::from("project-1")),
            source_archive: LogicalArtifactRef {
                artifact_id: ArtifactId::from("source-1"),
                normalized_relative_path: "source/plugin.nfsrc".into(),
                digest: digest('c'),
            },
            source_snapshot_digest: digest('d'),
            dependency_lock_digest: DigestHex::from("bad"),
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        });
        assert!(bundle.validate().is_err());
    }

    #[test]
    fn permanent_delete_cannot_persist_a_canceled_state() {
        let operation = ProductOperationRecord {
            operation_id: OperationId::from("delete-1"),
            kind: ProductOperationKind::MiniappPermanentDelete,
            owner: ProductOperationOwner::Miniapp {
                miniapp_id: MiniAppId::from("miniapp-1"),
            },
            state: ProductOperationState::Canceled,
            progress_percent: None,
            last_error: None,
            bounded_log_tail: Vec::new(),
            started_at_ms: 1,
            finished_at_ms: Some(2),
        };
        assert!(operation.validate().is_err());
    }

    #[test]
    fn platform_records_separate_required_and_optional_delivery() {
        let required = N1PlatformValidationRecord {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            cohort_id: ValidationCohortId::from("cohort-1"),
            source_commit: "a".repeat(40),
            stage: PlatformValidationStage::Candidate,
            cell_id: N1PlatformCellId::WindowsDesktopX64,
            requirement: PlatformCellRequirement::Required,
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            status: PlatformValidationStatus::Pass,
            not_delivered_reason: None,
            artifact_digests: BTreeMap::from([("package".into(), digest('b'))]),
            check_ids: BTreeSet::from(["windows_candidate".into()]),
        };
        required.validate().unwrap();

        let optional = N1PlatformValidationRecord {
            cell_id: N1PlatformCellId::LinuxHeadlessX64,
            requirement: PlatformCellRequirement::Optional,
            status: PlatformValidationStatus::NotDelivered,
            not_delivered_reason: Some(
                PlatformNotDeliveredReason::NotInReleaseScope,
            ),
            artifact_digests: BTreeMap::new(),
            check_ids: BTreeSet::new(),
            ..required
        };
        optional.validate().unwrap();
    }

    #[test]
    fn stable_promotion_requires_both_stages_for_every_required_cell() {
        let mut cohort = N1CohortLock {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            cohort_id: ValidationCohortId::from("cohort-1"),
            source_commit: "a".repeat(40),
            input_digests: BTreeMap::from([(
                "plugin_n1_contract".into(),
                digest('b'),
            )]),
            cells: BTreeMap::from([(
                N1PlatformCellId::WindowsDesktopX64,
                N1CohortCellLock {
                    requirement: PlatformCellRequirement::Required,
                    validation_manifest_digests: BTreeMap::from([(
                        PlatformValidationStage::Candidate,
                        digest('c'),
                    )]),
                },
            )]),
        };
        cohort.validate().unwrap();
        assert!(cohort.validate_stable_promotion().is_err());

        for cell_id in N1PlatformCellId::REQUIRED {
            cohort.cells.insert(
                cell_id,
                N1CohortCellLock {
                    requirement: PlatformCellRequirement::Required,
                    validation_manifest_digests: BTreeMap::from([
                        (PlatformValidationStage::Candidate, digest('d')),
                        (PlatformValidationStage::SignedRc, digest('e')),
                    ]),
                },
            );
        }
        cohort.validate_stable_promotion().unwrap();
    }
}
