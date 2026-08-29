//! Public API DTOs for the Agent Capability Platform control plane.
//!
//! Domain identity, digest validation, and immutable Preset/Snapshot semantics
//! remain owned by `nomifun-agent-contracts`. These DTOs are the HTTP wire
//! projection consumed by the product UI and transport adapters.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPresetSourceDto {
    Official,
    User,
    Package,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OfficialPresetKeyDto {
    #[serde(rename = "chat.minimal")]
    ChatMinimal,
    #[serde(rename = "assistant.general")]
    AssistantGeneral,
    #[serde(rename = "coding.codex")]
    CodingCodex,
    #[serde(rename = "companion.default")]
    CompanionDefault,
    #[serde(rename = "robot.default")]
    RobotDefault,
    #[serde(rename = "customer-service.default")]
    CustomerServiceDefault,
    #[serde(rename = "creative-studio.default")]
    CreativeStudioDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactCatalogRefDto {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetRevisionRefDto {
    pub preset_id: String,
    pub revision: u64,
    pub revision_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSnapshotRefDto {
    pub snapshot_id: String,
    pub snapshot_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedResourceBindingDto {
    pub binding_id: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub owner_id: String,
    pub operations: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_config_ref: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub typed_parameters: BTreeMap<String, String>,
}

/// Canonical binding value reused by every product target and RemoteBinding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingValueDto {
    pub preset_revision_ref: PresetRevisionRefDto,
    pub resolved_snapshot_ref: ResolvedSnapshotRefDto,
    pub typed_resource_bindings: Vec<TypedResourceBindingDto>,
    pub binding_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityExposureDto {
    Advertised,
    Discoverable,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySelectionDto {
    pub capability: ExactCatalogRefDto,
    pub required: bool,
    pub exposure: CapabilityExposureDto,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub action_allowlist: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub destination_constraints: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_budget_override: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_budget_override: Option<u32>,
    pub config: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetDocumentDto {
    pub schema_version: String,
    pub surfaces: BTreeSet<String>,
    pub model_route_refs: BTreeMap<String, String>,
    pub initial_capabilities: Vec<CapabilitySelectionDto>,
    pub on_demand_capabilities: Vec<CapabilitySelectionDto>,
    pub skill_bindings: Vec<ExactCatalogRefDto>,
    pub resource_bindings: Vec<TypedResourceBindingDto>,
    pub persona: String,
    pub instructions: String,
    pub context_policy: Value,
    pub execution_constraints: Value,
    pub runtime_budget: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetDraftDto {
    pub preset_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Request/editor-only provenance used while expanding an official template.
    /// It is never part of AgentPreset, Revision, Snapshot, or binding storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_template_key: Option<OfficialPresetKeyDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<PresetRevisionRefDto>,
    pub document: AgentPresetDocumentDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetSummaryDto {
    pub preset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    pub source: AgentPresetSourceDto,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_stable_revision: Option<PresetRevisionRefDto>,
    pub bound_target_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDefaultBindingPolicyDto {
    RequireExplicitSelection,
    SelectOnlyOwnedResource,
    LeaveUnbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedResourceDefaultDto {
    pub slot_key: String,
    pub resource_kind: String,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: ResourceDefaultBindingPolicyDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetSeedDto {
    pub initial_capabilities: Vec<ExactCatalogRefDto>,
    pub on_demand_capabilities: Vec<ExactCatalogRefDto>,
    pub skill_bindings: Vec<ExactCatalogRefDto>,
    pub typed_resource_defaults: Vec<TypedResourceDefaultDto>,
    pub required_runtime_features: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetRoleCoverageDto {
    pub required_capability_categories: BTreeSet<String>,
    pub required_capability_ids: BTreeSet<String>,
    pub required_runtime_features: BTreeSet<String>,
    pub required_resource_kinds: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetTemplateDto {
    pub template_key: OfficialPresetKeyDto,
    pub seed: OfficialPresetSeedDto,
    pub role_coverage: OfficialPresetRoleCoverageDto,
    pub immutable: bool,
    pub forkable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingSummaryDto {
    pub target_kind: String,
    pub target_id: String,
    pub preset_revision_ref: PresetRevisionRefDto,
    pub resolved_snapshot_ref: ResolvedSnapshotRefDto,
    pub binding_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshStartPresentationDto {
    pub data_generation: u32,
    pub legacy_data_imported: bool,
    pub official_template_count: u32,
    pub user_preset_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetLibraryResponse {
    pub official_templates: Vec<OfficialPresetTemplateDto>,
    pub user_presets: Vec<AgentPresetSummaryDto>,
    pub active_bindings: Vec<AgentBindingSummaryDto>,
    pub fresh_start: FreshStartPresentationDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogMaterializationStateDto {
    Materialized,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogItemDto {
    pub capability: ExactCatalogRefDto,
    pub kind: String,
    pub display_name: String,
    pub description: String,
    pub source_package: ExactCatalogRefDto,
    pub source_kind: String,
    pub materialization_state: CatalogMaterializationStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_code: Option<String>,
    pub supported_surfaces: BTreeSet<String>,
    pub required_runtime_features: BTreeSet<String>,
    pub required_resource_kinds: BTreeSet<String>,
    pub required_capabilities: Vec<ExactCatalogRefDto>,
    pub conflicting_capabilities: Vec<ExactCatalogRefDto>,
    pub action_count: u32,
    pub context_contributor_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogItemDto {
    pub skill: ExactCatalogRefDto,
    pub display_name: String,
    pub description: String,
    pub source_package: ExactCatalogRefDto,
    pub source_kind: String,
    pub required_capabilities: Vec<ExactCatalogRefDto>,
    pub supported_surfaces: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolCatalogItemDto {
    pub server_id: String,
    pub canonical_tool_key: String,
    pub capability: ExactCatalogRefDto,
    pub source_package: ExactCatalogRefDto,
    pub schema_digest: String,
    pub materialization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogResponse {
    pub capabilities: Vec<CapabilityCatalogItemDto>,
    pub skills: Vec<SkillCatalogItemDto>,
    pub mcp_tools: Vec<McpToolCatalogItemDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatusDto {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewDiagnosticSeverityDto {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewDiagnosticDto {
    pub severity: PreviewDiagnosticSeverityDto,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewCapabilityDto {
    pub capability: ExactCatalogRefDto,
    pub display_name: String,
    pub source_package: ExactCatalogRefDto,
    pub dependency_path: Vec<String>,
    pub required_runtime_features: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSummaryDto {
    pub initial_count: u32,
    pub on_demand_count: u32,
    pub active_at_start_count: u32,
    pub model_tool_count: u32,
    pub context_contributor_count: u32,
    pub on_demand_index_count: u32,
    pub skill_count: u32,
    pub mcp_count: u32,
    pub resource_binding_count: u32,
    pub provider_initialization_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionDiffDto {
    pub added_initial: BTreeSet<String>,
    pub removed_initial: BTreeSet<String>,
    pub added_on_demand: BTreeSet<String>,
    pub removed_on_demand: BTreeSet<String>,
    pub added_skills: BTreeSet<String>,
    pub removed_skills: BTreeSet<String>,
    pub resource_bindings_changed: bool,
    pub model_routes_changed: bool,
    pub instructions_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotInspectorDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<ResolvedSnapshotRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_revision_ref: Option<PresetRevisionRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile: Option<String>,
    pub required_runtime_protocol_version: String,
    pub required_runtime_features: BTreeSet<String>,
    pub initial_capabilities: Vec<PreviewCapabilityDto>,
    pub on_demand_capabilities: Vec<PreviewCapabilityDto>,
    pub compact_on_demand_index: Vec<String>,
    pub tool_schema_refs: Vec<String>,
    pub context_schema_refs: Vec<String>,
    pub mcp_materializations: Vec<McpToolCatalogItemDto>,
    pub typed_resource_bindings: Vec<TypedResourceBindingDto>,
    pub service_key_diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveAgentPresetPreviewRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_current_revision: Option<PresetRevisionRefDto>,
    pub draft: AgentPresetDraftDto,
    pub scene: String,
    pub surface: String,
    pub audience: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveSavedRevisionPreviewRequest {
    pub scene: String,
    pub surface: String,
    pub audience: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveAgentPresetPreviewResponse {
    pub status: PreviewStatusDto,
    pub draft_digest: String,
    pub preview_digest: String,
    pub candidate_revision_ref: PresetRevisionRefDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_snapshot_ref: Option<ResolvedSnapshotRefDto>,
    pub summary: PreviewSummaryDto,
    pub diagnostics: Vec<PreviewDiagnosticDto>,
    pub revision_diff: RevisionDiffDto,
    pub inspector: SnapshotInspectorDto,
    pub can_save_revision: bool,
    pub can_create_session: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentPresetRequest {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_from_revision: Option<PresetRevisionRefDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentPresetFromTemplateRequest {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub resource_bindings: Vec<TemplateResourceSelectionDto>,
    #[serde(default)]
    pub model_route_refs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateResourceSelectionDto {
    pub slot_key: String,
    pub resource_kind: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_config_ref: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub typed_parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetEditorResponse {
    pub preset: AgentPresetSummaryDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<AgentPresetRevisionDto>,
    pub draft: AgentPresetDraftDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevisionDto {
    pub reference: PresetRevisionRefDto,
    pub document: AgentPresetDocumentDto,
    pub created_by: String,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAgentPresetRevisionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_current_revision: Option<PresetRevisionRefDto>,
    pub preview_digest: String,
    pub draft: AgentPresetDraftDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAgentPresetRevisionResponse {
    pub preset: AgentPresetSummaryDto,
    pub revision: AgentPresetRevisionDto,
    pub resolved_snapshot_ref: ResolvedSnapshotRefDto,
    pub preview_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingTargetDto {
    pub target_kind: String,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingRecordDto {
    pub target: AgentBindingTargetDto,
    pub owner_user_id: String,
    pub agent_binding: AgentBindingValueDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutAgentBindingRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding_version: Option<u64>,
    pub agent_binding: AgentBindingValueDto,
}

/// Exact RemoteBinding projection. Remote-specific fields are only these four.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteBindingDto {
    pub remote_binding_id: String,
    pub owner_user_id: String,
    pub name: String,
    pub agent_binding: AgentBindingValueDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRemoteBindingRequest {
    pub name: String,
    pub agent_binding: AgentBindingValueDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRemoteBindingRequest {
    pub expected_binding_version: u64,
    pub expected_agent_binding_digest: String,
    pub name: String,
    pub agent_binding: AgentBindingValueDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteOpenRequestDto {
    pub binding_id: String,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteOpenResponseDto {
    pub agent_session_id: String,
    pub agent_binding: AgentBindingValueDto,
    pub open_state: RemoteOpenStateViewDto,
    pub cursor: SessionCursorDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteOpenStateViewDto {
    Opening,
    Ready,
    Failed {
        code: String,
        recoverable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTurnRequestDto {
    pub agent_session_id: String,
    pub input: Value,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteObserveRequestDto {
    pub agent_session_id: String,
    pub after_cursor: SessionCursorDto,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteCancelRequestDto {
    pub agent_session_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCursorDto {
    pub agent_session_id: String,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteObserveResponseDto {
    pub agent_session_id: String,
    pub events: Vec<Value>,
    pub messages: Vec<Value>,
    pub next_cursor: SessionCursorDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteMutationResponseDto {
    pub agent_session_id: String,
    pub cursor: SessionCursorDto,
    pub session_status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorDraftStateDto {
    Clean,
    Dirty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorRevisionActionDto {
    ReuseCurrentRevision,
    SaveOrdinaryVisibleRevision,
}

/// Client-side D-022 plan. It is not an API endpoint or alternate Session mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetEditorTestPlanDto {
    pub draft_state: EditorDraftStateDto,
    pub revision_action: EditorRevisionActionDto,
    pub preview: ResolveAgentPresetPreviewResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_request: Option<SaveAgentPresetRevisionRequest>,
    pub session_create_path: String,
    pub uses_real_typed_resources: bool,
    pub uses_full_auto: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionRequestDto {
    pub agent_binding: AgentBindingValueDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionResponseDto {
    pub agent_session_id: String,
    pub agent_binding: AgentBindingValueDto,
    pub state: String,
    pub cursor: SessionCursorDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionTurnRequestDto {
    pub input: Value,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionTurnResponseDto {
    pub agent_session_id: String,
    pub operation_id: String,
    pub cursor: SessionCursorDto,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkAgentSessionRequestDto {
    pub target_agent_binding: AgentBindingValueDto,
    pub parent_through_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkAgentSessionResponseDto {
    pub parent_agent_session_id: String,
    pub child_agent_session_id: String,
    pub child_agent_binding: AgentBindingValueDto,
    pub parent_through_seq: u64,
    pub child_base_is_self_contained: bool,
    pub copies_full_transcript: bool,
    pub migrates_runtime_private_handles: bool,
    pub replays_tool_or_effect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotCompatibilityViewDto {
    CompatibleExact {
        runtime_release_digest: String,
        hello_payload_digest: String,
    },
    ExecutorUnavailable {
        error_code: String,
        mismatches: Vec<SnapshotContractMismatchDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContractMismatchDto {
    pub kind: String,
    pub subject: String,
    pub expected: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionContinuationViewDto {
    pub agent_session_id: String,
    pub compatibility: SnapshotCompatibilityViewDto,
    pub history_read_only: bool,
    pub can_continue_same_session: bool,
    pub requires_explicit_fork: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_request: Option<ForkAgentSessionRequestDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationTokenStatusDto {
    Unconfigured,
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteCredentialContinuationDto {
    pub requires_same_owner: bool,
    pub requires_explicit_agent_session_id: bool,
    pub implicit_session_lookup: bool,
    pub auth_error_code: String,
    pub rest_status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationTokenStateResponseDto {
    pub status: InstallationTokenStatusDto,
    pub configured: bool,
    pub continuation: RemoteCredentialContinuationDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateInstallationTokenResponseDto {
    pub access_token: String,
    pub status: InstallationTokenStatusDto,
    pub shown_once: bool,
    pub existing_sessions_unchanged: bool,
    pub continuation: RemoteCredentialContinuationDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInstallationTokenResponseDto {
    pub status: InstallationTokenStatusDto,
    pub existing_sessions_unchanged: bool,
    pub admitted_operations_continue_to_finite_boundary: bool,
    pub continuation: RemoteCredentialContinuationDto,
}
