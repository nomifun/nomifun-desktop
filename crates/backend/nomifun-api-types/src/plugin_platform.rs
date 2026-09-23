//! Product-facing DTOs for Unified Plugin Core.
//!
//! This module is the sole Plugin HTTP contract. It exposes one local Plugin
//! identity, content-addressed artifacts, managed Drafts, Action/Binding
//! summaries, and one Surface Bridge. Process authority and credential values
//! remain Host-owned and are never accepted on this wire.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn empty_object() -> Value {
    serde_json::json!({})
}

// ---------------------------------------------------------------------------
// Manifest projection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginServiceModeDto {
    OnDemand,
    Continuous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntrypointsSummaryDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_mode: Option<PluginServiceModeDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginActionEffectDto {
    Read,
    Write,
    External,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginActionSummaryDto {
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effect: PluginActionEffectDto,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum PluginBindingPointDto {
    #[serde(rename = "agent.tool")]
    AgentTool,
    #[serde(rename = "agent.context")]
    AgentContext,
    #[serde(rename = "agent.before_model")]
    AgentBeforeModel,
    #[serde(rename = "agent.before_tool")]
    AgentBeforeTool,
    #[serde(rename = "desktop.command")]
    DesktopCommand,
    #[serde(rename = "desktop.event")]
    DesktopEvent,
    #[serde(rename = "automation.action")]
    AutomationAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginBindingSummaryDto {
    pub point: PluginBindingPointDto,
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_stable_id: Option<String>,
    #[serde(default)]
    pub optional: bool,
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDesktopCommandDto {
    pub action_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: PluginActionEffectDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvokePluginDesktopCommandRequest {
    pub action_id: String,
    #[serde(default = "empty_object")]
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchPluginDesktopEventRequest {
    #[serde(default = "empty_object")]
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDesktopEventOutputDto {
    pub action_id: String,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDesktopEventFailureDto {
    pub action_id: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDesktopEventReportDto {
    pub outputs: Vec<PluginDesktopEventOutputDto>,
    pub failures: Vec<PluginDesktopEventFailureDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestSummaryDto {
    pub schema: String,
    pub package_id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub host_api: String,
    pub entrypoints: PluginEntrypointsSummaryDto,
    #[serde(default)]
    pub actions: Vec<PluginActionSummaryDto>,
    #[serde(default)]
    pub bindings: Vec<PluginBindingSummaryDto>,
    pub data_version: u32,
    pub config_schema: Value,
    #[serde(default)]
    pub secret_slots: Vec<String>,
    #[serde(default)]
    pub permissions: BTreeSet<String>,
}

// ---------------------------------------------------------------------------
// Installed Plugin state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginArtifactDataPointerDto {
    pub artifact_digest: String,
    pub package_version: String,
    pub data_generation: String,
    pub data_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginObservedStateDto {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeObservationDto {
    pub state: PluginObservedStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSummaryDto {
    pub plugin_id: String,
    pub package_id: String,
    pub display_name: String,
    pub description: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at_ms: Option<u64>,
    pub revision: u64,
    pub active: PluginArtifactDataPointerDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PluginArtifactDataPointerDto>,
    pub has_ui: bool,
    pub has_service: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_mode: Option<PluginServiceModeDto>,
    pub action_count: u32,
    pub binding_count: u32,
    pub runtime: PluginRuntimeObservationDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigDto {
    pub schema: Value,
    pub values: Value,
    pub valid: bool,
    #[serde(default)]
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCredentialBindingStatusDto {
    Unbound,
    Bound,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCredentialBindingDto {
    pub slot: String,
    pub required: bool,
    pub status: PluginCredentialBindingStatusDto,
    /// Host Credential Store identity only. Credential values never cross this API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginGrantDto {
    pub permission: String,
    pub granted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDetailDto {
    pub summary: PluginSummaryDto,
    pub manifest: PluginManifestSummaryDto,
    pub config: PluginConfigDto,
    #[serde(default)]
    pub credential_bindings: Vec<PluginCredentialBindingDto>,
    #[serde(default)]
    pub grants: Vec<PluginGrantDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLibraryResponseDto {
    pub revision: u64,
    #[serde(default)]
    pub plugins: Vec<PluginSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLibraryItemDto {
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<String>,
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLibraryStateDto {
    pub revision: u64,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub items: Vec<PluginLibraryItemDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCredentialReferenceKindDto {
    Provider,
    Connection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCredentialReferenceDto {
    pub credential_id: String,
    pub kind: PluginCredentialReferenceKindDto,
    pub label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePluginLibraryStateRequest {
    pub expected_revision: u64,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub items: Vec<PluginLibraryItemDto>,
}

// ---------------------------------------------------------------------------
// Draft authoring and preview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDraftStatusDto {
    Ready,
    Generating,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftSummaryDto {
    pub draft_id: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_plugin_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub status: PluginDraftStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDraftMessageRoleDto {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftMessageDto {
    pub role: PluginDraftMessageRoleDto,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftFileDto {
    pub path: String,
    pub media_type: String,
    pub digest: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftDetailDto {
    pub summary: PluginDraftSummaryDto,
    #[serde(default)]
    pub messages: Vec<PluginDraftMessageDto>,
    #[serde(default)]
    pub files: Vec<PluginDraftFileDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftListResponseDto {
    #[serde(default)]
    pub drafts: Vec<PluginDraftSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePluginDraftRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plugin_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratePluginDraftRequest {
    pub expected_revision: u64,
    pub provider_id: String,
    pub model: String,
    pub requirement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelPluginDraftGenerationRequest {
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacePluginDraftFileRequest {
    pub expected_revision: u64,
    pub path: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginDraftFileRequest {
    pub expected_revision: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginPreviewAccessRequest {
    #[serde(default)]
    pub permissions: BTreeSet<String>,
    /// Slot -> Host Credential Store identity. No credential value is accepted.
    #[serde(default)]
    pub credential_bindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPluginDraftRequest {
    pub expected_revision: u64,
    pub config: Value,
    #[serde(default)]
    pub access: PluginPreviewAccessRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavePluginDraftRequest {
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plugin_revision: Option<u64>,
    /// Opaque, Host-issued confirmation bound to the staged bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_confirmation_id: Option<String>,
    pub config: Value,
    #[serde(default)]
    pub credential_bindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginDraftRequest {
    pub expected_revision: u64,
}

// ---------------------------------------------------------------------------
// Import, install and permission confirmation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginImportKindDto {
    Directory,
    Zip,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectPluginImportRequest {
    pub source_path: String,
    pub kind: PluginImportKindDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPluginImportRequest {
    pub source_path: String,
    pub kind: PluginImportKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plugin_revision: Option<u64>,
    #[serde(default)]
    pub create_copy: bool,
    /// Opaque, Host-issued confirmation bound to the freshly staged bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_confirmation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_bindings: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginBackupDataSummaryDto {
    pub includes_data: bool,
    pub data_version: u32,
    pub database_size_bytes: u64,
    pub file_count: u64,
    pub includes_config: bool,
    #[serde(default)]
    pub credential_slots_to_rebind: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissionExpansionDto {
    pub confirmation_id: String,
    #[serde(default)]
    pub added_permissions: BTreeSet<String>,
    #[serde(default)]
    pub added_secret_slots: BTreeSet<String>,
    pub trusted_local_service: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginImportInspectionDto {
    pub kind: PluginImportKindDto,
    /// Computed by the Host from staged bytes; never supplied by the caller.
    pub artifact_digest: String,
    pub manifest: PluginManifestSummaryDto,
    pub trusted_local_service: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_plugin_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PluginBackupDataSummaryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_expansion: Option<PluginPermissionExpansionDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginInstallOutcomeDto {
    Installed { plugin: Box<PluginDetailDto> },
    ConfirmationRequired {
        confirmation: PluginPermissionExpansionDto,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPluginImportResponseDto {
    pub result: PluginInstallOutcomeDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavePluginDraftResponseDto {
    pub draft: PluginDraftSummaryDto,
    pub result: PluginInstallOutcomeDto,
}

// ---------------------------------------------------------------------------
// Installed Plugin commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginEnabledRequest {
    pub expected_revision: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePluginRequest {
    pub expected_revision: u64,
    pub config: Value,
    /// Slot -> Host Credential Store identity, or null to unbind.
    #[serde(default)]
    pub credential_bindings: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub grants: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRestoreModeDto {
    PreviousCode,
    PreviousCodeAndData,
    FromTrash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePluginRequest {
    pub expected_revision: u64,
    pub mode: PluginRestoreModeDto,
    #[serde(default)]
    pub acknowledge_data_loss: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashPluginRequest {
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginRequest {
    pub expected_revision: u64,
    pub acknowledge_permanent_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPluginPackageRequest {
    pub expected_revision: u64,
    pub destination_path: String,
    #[serde(default)]
    pub include_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPluginBackupRequest {
    pub expected_revision: u64,
    pub destination_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginExportResultDto {
    pub destination_path: String,
    pub digest: String,
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------------
// Surface and preview descriptors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSurfaceDescriptorDto {
    /// Present for an installed Plugin and for a Draft editing that Plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    pub artifact_digest: String,
    pub surface_session_id: String,
    pub surface_generation: u64,
    pub entrypoint: String,
    pub is_preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDraftPreviewResponseDto {
    pub draft_revision: u64,
    pub descriptor: PluginSurfaceDescriptorDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenPluginSurfaceRequest {
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosePluginSurfaceRequest {
    pub surface_session_id: String,
    pub surface_generation: u64,
}

// ---------------------------------------------------------------------------
// Unified Surface Bridge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginKvRequestDto {
    Get { key: String },
    Set { key: String, value: Value },
    Delete { key: String },
    CompareAndSwap {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDatabaseStatementDto {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginDatabaseRequestDto {
    Query {
        sql: String,
        #[serde(default)]
        parameters: Vec<Value>,
    },
    Execute {
        sql: String,
        #[serde(default)]
        parameters: Vec<Value>,
    },
    Batch {
        statements: Vec<PluginDatabaseStatementDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginFilesRequestDto {
    Read { path: String },
    Write {
        path: String,
        content_base64: String,
        #[serde(default)]
        overwrite: bool,
    },
    List {
        #[serde(default)]
        path: String,
    },
    Delete { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginCacheRequestDto {
    Get { key: String },
    Set {
        key: String,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u64>,
    },
    Delete { key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeTargetDto {
    Kv { request: PluginKvRequestDto },
    Db { request: PluginDatabaseRequestDto },
    Files { request: PluginFilesRequestDto },
    Cache { request: PluginCacheRequestDto },
    Actions { action: String, input: Value },
    Host { capability: String, input: Value },
    Config,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginBridgeRequestDto {
    pub call_id: String,
    pub target: PluginBridgeTargetDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchPluginBridgeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    pub artifact_digest: String,
    pub surface_session_id: String,
    pub surface_generation: u64,
    pub is_preview: bool,
    pub request: PluginBridgeRequestDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginKvResultDto {
    Value {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    Written { revision: u64 },
    Deleted { existed: bool },
    CompareAndSwap {
        applied: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_revision: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginDatabaseResultDto {
    Rows { rows: Vec<BTreeMap<String, Value>> },
    Executed {
        rows_affected: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_insert_rowid: Option<i64>,
    },
    Batch { results: Vec<PluginDatabaseResultDto> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginFileEntryDto {
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginFilesResultDto {
    Data { content_base64: String },
    Written { size_bytes: u64 },
    Entries { entries: Vec<PluginFileEntryDto> },
    Deleted { existed: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginCacheResultDto {
    Value {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
    Stored,
    Deleted { existed: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeSuccessDto {
    Kv { result: PluginKvResultDto },
    Db { result: PluginDatabaseResultDto },
    Files { result: PluginFilesResultDto },
    Cache { result: PluginCacheResultDto },
    Actions { result: Value },
    Host { result: Value },
    Config { config: Value },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginBridgeErrorDto {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub outcome_unknown: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeResultDto {
    Success {
        call_id: String,
        result: PluginBridgeSuccessDto,
    },
    Failure {
        call_id: String,
        error: PluginBridgeErrorDto,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn draft_status_has_one_persisted_wire_vocabulary() {
        for (status, wire) in [
            (PluginDraftStatusDto::Ready, "ready"),
            (PluginDraftStatusDto::Generating, "generating"),
            (PluginDraftStatusDto::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        }
        for retired in ["interrupted", "saving"] {
            assert!(serde_json::from_value::<PluginDraftStatusDto>(json!(retired)).is_err());
        }
    }

    #[test]
    fn manifest_projection_uses_inline_schemas_and_stable_actions() {
        let dto = PluginManifestSummaryDto {
            schema: "nomifun.plugin/v1".into(),
            package_id: "local.todo".into(),
            version: "1.0.0".into(),
            name: "Todo".into(),
            description: "Todo app".into(),
            host_api: ">=1 <2".into(),
            entrypoints: PluginEntrypointsSummaryDto {
                ui: Some("ui/index.html".into()),
                service: Some("service/main.mjs".into()),
                service_mode: Some(PluginServiceModeDto::OnDemand),
            },
            actions: vec![PluginActionSummaryDto {
                action_id: "add_task".into(),
                stable_id: Some("plugin:plugin-1/add_task".into()),
                name: "Add task".into(),
                description: "Adds a task".into(),
                input_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                effect: PluginActionEffectDto::Write,
            }],
            bindings: vec![PluginBindingSummaryDto {
                point: PluginBindingPointDto::AgentTool,
                action_id: "add_task".into(),
                action_stable_id: Some("plugin:plugin-1/add_task".into()),
                optional: false,
                supported: true,
                unavailable_reason: None,
            }],
            data_version: 1,
            config_schema: json!({"type":"object"}),
            secret_slots: vec!["api_key".into()],
            permissions: BTreeSet::from(["network".into()]),
        };
        let value = serde_json::to_value(dto).unwrap();
        assert_eq!(value["entrypoints"]["service_mode"], "on_demand");
        assert_eq!(value["bindings"][0]["point"], "agent.tool");
        assert_eq!(value["actions"][0]["input_schema"]["type"], "object");
        assert!(value.get("schemas").is_none());
    }

    #[test]
    fn import_requests_accept_source_and_kind_but_never_a_caller_digest() {
        let request: InstallPluginImportRequest = serde_json::from_value(json!({
            "source_path":"C:/plugins/todo.zip",
            "kind":"zip",
            "expected_plugin_revision":7,
            "create_copy":false
        }))
        .unwrap();
        assert_eq!(request.kind, PluginImportKindDto::Zip);
        for legacy in [
            "expected_digest",
            "expected_artifact_digest",
            "expected_bundle_digest",
            "release_id",
        ] {
            let mut value = json!({"source_path":"C:/plugins/todo.zip","kind":"zip"});
            value.as_object_mut().unwrap().insert(legacy.into(), json!("x"));
            assert!(serde_json::from_value::<InstallPluginImportRequest>(value).is_err());
        }
        assert!(
            serde_json::from_value::<InspectPluginImportRequest>(json!({
                "source_path":"C:/plugins/todo.zip",
                "kind":"zip",
                "expected_digest":"caller-controlled"
            }))
            .is_err()
        );
    }

    #[test]
    fn command_requests_reject_retired_identity_and_pointer_fields() {
        assert!(serde_json::from_value::<SetPluginEnabledRequest>(json!({
            "expected_revision":3,
            "enabled":true,
            "mount_id":"old"
        })).is_err());
        assert!(serde_json::from_value::<SetPluginEnabledRequest>(json!({
            "expected_revision":3,
            "enabled":true,
            "expected_pointer_revision":2
        })).is_err());
        assert!(serde_json::from_value::<SavePluginDraftRequest>(json!({
            "expected_revision":3,
            "acknowledge_service_test":{"receipt_id":"old"}
        })).is_err());
    }

    #[test]
    fn credential_inputs_are_store_references_not_secret_values() {
        let request = ConfigurePluginRequest {
            expected_revision: 4,
            config: json!({"theme":"dark"}),
            credential_bindings: BTreeMap::from([(
                "api_key".into(),
                Some("credential-0190".into()),
            )]),
            grants: BTreeMap::new(),
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["credential_bindings"]["api_key"], "credential-0190");
        assert!(value.get("credentials").is_none());
        assert!(value.get("secret_values").is_none());
    }

    #[test]
    fn one_surface_descriptor_fences_installed_and_preview_sessions() {
        let value = serde_json::to_value(PluginSurfaceDescriptorDto {
            plugin_id: Some("plugin-1".into()),
            draft_id: Some("draft-1".into()),
            artifact_digest: "a".repeat(64),
            surface_session_id: "surface-1".into(),
            surface_generation: 7,
            entrypoint: "ui/index.html".into(),
            is_preview: true,
        })
        .unwrap();
        assert_eq!(value["is_preview"], true);
    }

    #[test]
    fn bridge_has_one_target_set_and_rejects_old_side_channels() {
        let request: PluginBridgeRequestDto = serde_json::from_value(json!({
            "call_id":"call-1",
            "target":{
                "target":"actions",
                "action":"plugin:plugin-1/add_task",
                "input":{"title":"Ship"}
            }
        }))
        .unwrap();
        assert!(matches!(request.target, PluginBridgeTargetDto::Actions { .. }));
        for target in [
            json!({"target":"kv","request":{"operation":"get","key":"k"}}),
            json!({"target":"db","request":{"operation":"query","sql":"SELECT 1","parameters":[]}}),
            json!({"target":"files","request":{"operation":"list","path":""}}),
            json!({"target":"cache","request":{"operation":"get","key":"k"}}),
            json!({"target":"actions","action":"plugin:plugin-1/a","input":{}}),
            json!({"target":"host","capability":"desktop.files.open","input":{}}),
            json!({"target":"config"}),
        ] {
            assert!(serde_json::from_value::<PluginBridgeTargetDto>(target).is_ok());
        }
        for result in [
            json!({"target":"kv","result":{"outcome":"value"}}),
            json!({"target":"db","result":{"outcome":"rows","rows":[]}}),
            json!({"target":"files","result":{"outcome":"entries","entries":[]}}),
            json!({"target":"cache","result":{"outcome":"stored"}}),
            json!({"target":"actions","result":{}}),
            json!({"target":"host","result":{}}),
            json!({"target":"config","config":{}}),
        ] {
            assert!(serde_json::from_value::<PluginBridgeSuccessDto>(result).is_ok());
        }
        for retired in ["host_kv", "service", "agent_session"] {
            assert!(serde_json::from_value::<PluginBridgeRequestDto>(json!({
                "call_id":"call-1",
                "target":{"target":retired,"request":{}}
            })).is_err());
        }
    }

    #[test]
    fn request_structs_are_closed_to_unknown_fields() {
        assert!(serde_json::from_value::<PreviewPluginDraftRequest>(json!({
            "expected_revision":1,
            "config":{},
            "access":{"permissions":[],"credential_bindings":{}},
            "publish":true
        })).is_err());
        assert!(serde_json::from_value::<ExportPluginBackupRequest>(json!({
            "expected_revision":1,
            "destination_path":"C:/exports/todo.nomibackup",
            "include_credentials":true
        })).is_err());
        assert!(serde_json::from_value::<PluginKvRequestDto>(json!({
            "operation":"get",
            "key":"settings",
            "mount_revision":1
        })).is_err());
    }

    #[test]
    fn permission_expansion_uses_an_opaque_host_confirmation() {
        let value = serde_json::to_value(SavePluginDraftRequest {
            expected_revision: 9,
            expected_plugin_revision: Some(4),
            permission_confirmation_id: Some("confirmation-0190".into()),
            config: json!({"theme":"dark"}),
            credential_bindings: BTreeMap::from([(
                "api_key".into(),
                "provider:0190f5fe-7c00-7a00-8000-000000000001".into(),
            )]),
        })
        .unwrap();
        assert_eq!(value["permission_confirmation_id"], "confirmation-0190");
        assert!(value.get("approved_permissions").is_none());
        assert!(value.get("artifact_digest").is_none());
    }
}
