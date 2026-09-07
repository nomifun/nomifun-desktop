//! Product-facing DTOs for the Phase N1 Plugin Platform.
//!
//! These types intentionally project the machine contracts into user-visible
//! product state. Host IPC, process generations, commit fences, and raw
//! credential material are not part of this wire.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JavascriptRuntimeSourceDto {
    ManualPath,
    ProcessPath,
    Managed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JavascriptRuntimeCompatibilityDto {
    Recommended,
    Compatible,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavascriptRuntimeRefDto {
    pub runtime_installation_id: String,
    pub node_version: String,
    pub runtime_target: String,
    pub executable_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavascriptRuntimeProbeDto {
    pub source: JavascriptRuntimeSourceDto,
    pub executable_path: String,
    pub compatibility: JavascriptRuntimeCompatibilityDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<JavascriptRuntimeRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JavascriptRuntimeDownloadStateDto {
    NotInstalled,
    Downloading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavascriptRuntimeDownloadDto {
    pub download_revision: u64,
    pub state: JavascriptRuntimeDownloadStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<JavascriptRuntimeRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSwitchParticipantKindDto {
    PluginMount,
    MiniappService,
    BuildFoundation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSwitchParticipantStatusDto {
    Passed,
    Failed,
    NotCovered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSwitchParticipantDto {
    pub kind: RuntimeSwitchParticipantKindDto,
    pub owner_id: String,
    pub status: RuntimeSwitchParticipantStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavascriptRuntimeStatusDto {
    pub selection_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<JavascriptRuntimeRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_candidate: Option<JavascriptRuntimeRefDto>,
    #[serde(default)]
    pub probes: Vec<JavascriptRuntimeProbeDto>,
    #[serde(default)]
    pub switch_participants: Vec<RuntimeSwitchParticipantDto>,
    pub requires_switch_decision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub non_recommended_warning_acknowledged: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_offer: Option<JavascriptRuntimeDownloadOfferDto>,
    pub download: JavascriptRuntimeDownloadDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavascriptRuntimeDownloadOfferDto {
    pub offer_digest: String,
    pub node_version: String,
    pub runtime_target: String,
    pub archive_file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProbeJavascriptRuntimeRequest {
    AutoDiscover {
        expected_selection_revision: u64,
    },
    ManualPath {
        expected_selection_revision: u64,
        executable_path: String,
    },
    ManagedInstallation {
        expected_selection_revision: u64,
        runtime_installation_id: String,
        expected_executable_digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmJavascriptRuntimeDownloadRequest {
    pub expected_selection_revision: u64,
    pub expected_offer_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginJavascriptRuntimeSwitchRequest {
    pub expected_selection_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_selected_runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_selected_executable_digest: Option<String>,
    pub candidate_runtime_id: String,
    pub expected_candidate_executable_digest: String,
    pub acknowledge_non_recommended_runtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSwitchDecisionDto {
    CommitCandidate,
    AbortAndRestoreSelected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideJavascriptRuntimeSwitchRequest {
    pub expected_selection_revision: u64,
    pub candidate_runtime_id: String,
    pub expected_candidate_executable_digest: String,
    pub decision: RuntimeSwitchDecisionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTargetRefDto {
    pub package_id: String,
    pub package_version: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCandidateRefDto {
    pub candidate_id: String,
    pub candidate_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginApplyModeDto {
    AskBeforeApply,
    AutoCompatibleWhenIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleDto {
    Enabled,
    Disabled,
    UninstalledDataRetained,
    DeletePending,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginProjectSourceStateDto {
    Empty,
    Editable,
    RuntimeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSummaryDto {
    pub mount_id: String,
    pub mount_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub lifecycle: PluginLifecycleDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<PluginTargetRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PluginTargetRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_project_id: Option<String>,
    pub contribution_count: u32,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginProjectSummaryDto {
    pub project_id: String,
    pub project_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_mount_id: Option<String>,
    pub source_state: PluginProjectSourceStateDto,
    pub build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_candidate: Option<PluginCandidateRefDto>,
    pub apply_mode: PluginApplyModeDto,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLibraryResponseDto {
    pub library_revision: u64,
    pub plugins: Vec<PluginSummaryDto>,
    pub projects: Vec<PluginProjectSummaryDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginConsumerSurfaceDto {
    Agent,
    Gateway,
    Knowledge,
    Remote,
    Automation,
    Ui,
    MiniappService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginConsumerAvailabilityStatusDto {
    Active,
    Disabled,
    Unavailable,
    NeedsRuntime,
    ContractMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConsumerAvailabilityDto {
    pub surface: PluginConsumerSurfaceDto,
    pub status: PluginConsumerAvailabilityStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginContributionProvenanceDto {
    pub mount_id: String,
    pub mount_revision: u64,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub contribution_id: String,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCapabilityContributionDto {
    pub capability_id: String,
    pub capability_version: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub provenance: PluginContributionProvenanceDto,
    pub consumer_availability: Vec<PluginConsumerAvailabilityDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigSchemaDto {
    pub schema_digest: String,
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigStateDto {
    pub config_revision: u64,
    pub schema_digest: String,
    pub values: Value,
    pub valid: bool,
    #[serde(default)]
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialBindingStatusDto {
    Unbound,
    Bound,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSlotBindingDto {
    pub slot_key: String,
    pub display_name: String,
    pub required: bool,
    pub status: CredentialBindingStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCandidateOriginDto {
    Build,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompatibilityDto {
    Compatible,
    Breaking,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCandidateTestStatusDto {
    NotRun,
    Passed,
    Failed,
    NeedsTestInput,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCandidateTestDto {
    pub status: PluginCandidateTestStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<String>,
    pub candidate_id: String,
    pub candidate_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<JavascriptRuntimeRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_test_input_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAffectedConsumerDto {
    pub surface: PluginConsumerSurfaceDto,
    pub consumer_id: String,
    pub contribution_id: String,
    pub expected_contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCandidateImpactDto {
    pub compatibility: PluginCompatibilityDto,
    #[serde(default)]
    pub changed_contracts: Vec<String>,
    #[serde(default)]
    pub affected_consumers: Vec<PluginAffectedConsumerDto>,
    pub can_apply: bool,
    pub can_auto_apply: bool,
    #[serde(default)]
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyCandidateDto {
    pub candidate: PluginCandidateRefDto,
    pub origin: PluginCandidateOriginDto,
    pub target: PluginTargetRefDto,
    pub project_build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_target_digest: Option<String>,
    pub test: PluginCandidateTestDto,
    pub impact: PluginCandidateImpactDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDetailDto {
    pub summary: PluginSummaryDto,
    pub capabilities: Vec<PluginCapabilityContributionDto>,
    pub config_schema: PluginConfigSchemaDto,
    pub config: PluginConfigStateDto,
    pub credential_bindings_revision: u64,
    pub credential_slots: Vec<CredentialSlotBindingDto>,
    pub retained_data: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginProjectDetailDto {
    pub summary: PluginProjectSummaryDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_lock_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<PluginReadyCandidateDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_operation: Option<DurableOperationSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePluginProjectRequest {
    pub expected_library_revision: u64,
    pub package_id: String,
    pub package_version: String,
    pub display_name: String,
    pub description: String,
    pub language: PluginProjectLanguageDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_linked_mount_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_linked_target_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginProjectLanguageDto {
    JavaScript,
    TypeScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginImportKindDto {
    PrebuiltArtifact,
    ShareBundle,
    SourceBundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPluginRequest {
    pub expected_library_revision: u64,
    pub import_kind: PluginImportKindDto,
    pub source_path: String,
    pub expected_bundle_or_artifact_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_project_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPluginProjectRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub expected_source_snapshot_digest: String,
    pub expected_dependency_lock_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestPluginCandidateRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub candidate_id: String,
    pub expected_candidate_digest: String,
    pub expected_config_revision: u64,
    pub expected_credential_bindings_revision: u64,
    pub resolved_test_input_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplyPluginTargetDto {
    InitialInstall {
        expected_library_revision: u64,
    },
    ExistingMount {
        mount_id: String,
        expected_mount_revision: u64,
        expected_current_target_digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPluginCandidateRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub candidate_id: String,
    pub expected_candidate_digest: String,
    pub target: ApplyPluginTargetDto,
    pub allow_breaking: bool,
    pub acknowledge_test_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardPluginCandidateRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub candidate_id: String,
    pub expected_candidate_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginProjectRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_ready_candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_ready_candidate_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePluginRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: String,
    pub expected_config_revision: u64,
    pub expected_schema_digest: String,
    pub values: Value,
    pub credential_bindings: BTreeMap<String, Option<String>>,
    pub expected_credential_bindings_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginEnabledRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPluginRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePluginPreviousRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: String,
    pub expected_previous_target_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallPluginRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_current_target_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginDataRequest {
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_lifecycle: PluginLifecycleDto,
    pub expected_data_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginAutoApplyRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_linked_mount_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_linked_target_digest: Option<String>,
    pub apply_mode: PluginApplyModeDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginShareSourceDto {
    ReadyCandidate,
    CurrentMount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePluginRequest {
    pub project_id: String,
    pub expected_project_revision: u64,
    pub source: PluginShareSourceDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_candidate_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_mount_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_target_digest: Option<String>,
    pub destination_path: String,
    pub include_source: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableOperationKindDto {
    Build,
    Import,
    Export,
    MiniappPermanentDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableOperationStateDto {
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "owner", rename_all = "snake_case", deny_unknown_fields)]
pub enum DurableOperationOwnerDto {
    PluginProject { project_id: String },
    PluginMount { mount_id: String },
    Miniapp { miniapp_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableOperationSummaryDto {
    pub operation_id: String,
    pub operation_revision: u64,
    pub kind: DurableOperationKindDto,
    pub owner: DurableOperationOwnerDto,
    pub state: DurableOperationStateDto,
    pub cancelable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableOperationDetailDto {
    pub summary: DurableOperationSummaryDto,
    #[serde(default)]
    pub bounded_log_tail: Vec<String>,
    #[serde(default)]
    pub result_artifact_digests: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelDurableOperationRequest {
    pub operation_id: String,
    pub expected_operation_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_no_secret_keys(value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    let normalized = key.to_ascii_lowercase();
                    assert!(!normalized.contains("secret"), "secret field leaked: {key}");
                    assert!(!normalized.contains("plaintext"), "plaintext field leaked: {key}");
                    assert_no_secret_keys(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    assert_no_secret_keys(value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn plugin_wire_serializes_snake_case_and_credential_references_only() {
        let request = ConfigurePluginRequest {
            mount_id: "mount-1".into(),
            expected_mount_revision: 4,
            expected_current_target_digest: "a".repeat(64),
            expected_config_revision: 2,
            expected_schema_digest: "b".repeat(64),
            values: json!({"endpoint": "https://example.invalid"}),
            credential_bindings: BTreeMap::from([(
                "provider".into(),
                Some("credential-1".into()),
            )]),
            expected_credential_bindings_revision: 3,
        };

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["expected_mount_revision"], 4);
        assert_eq!(
            value["credential_bindings"]["provider"],
            Value::String("credential-1".into())
        );
        assert_no_secret_keys(&value);
    }

    #[test]
    fn project_creation_keeps_package_identity_separate_from_display_metadata() {
        let value = serde_json::to_value(CreatePluginProjectRequest {
            expected_library_revision: 4,
            package_id: "dev.nomifun.csv-tools".into(),
            package_version: "0.1.0".into(),
            display_name: "CSV Tools".into(),
            description: "Read and transform CSV files.".into(),
            language: PluginProjectLanguageDto::TypeScript,
            linked_mount_id: None,
            expected_linked_mount_revision: None,
            expected_linked_target_digest: None,
        })
        .unwrap();
        assert_eq!(value["package_id"], "dev.nomifun.csv-tools");
        assert_eq!(value["display_name"], "CSV Tools");
        assert_eq!(value["language"], "type_script");
    }

    #[test]
    fn plugin_requests_reject_unknown_fields() {
        let error = serde_json::from_value::<ApplyPluginCandidateRequest>(json!({
            "project_id": "project-1",
            "expected_project_revision": 2,
            "expected_build_generation": 7,
            "candidate_id": "candidate-1",
            "expected_candidate_digest": "a".repeat(64),
            "target": {
                "target": "existing_mount",
                "mount_id": "mount-1",
                "expected_mount_revision": 5,
                "expected_current_target_digest": "b".repeat(64)
            },
            "allow_breaking": false,
            "acknowledge_test_warning": false,
            "legacy_extension_id": "must-fail"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn runtime_switch_decision_uses_exact_candidate_identity() {
        let value = serde_json::to_value(DecideJavascriptRuntimeSwitchRequest {
            expected_selection_revision: 9,
            candidate_runtime_id: "runtime-2".into(),
            expected_candidate_executable_digest: "c".repeat(64),
            decision: RuntimeSwitchDecisionDto::CommitCandidate,
        })
        .unwrap();

        assert_eq!(value["decision"], "commit_candidate");
        assert_eq!(value["expected_selection_revision"], 9);
        assert!(value.get("process_id").is_none());
        assert!(value.get("host_generation").is_none());
    }
}
