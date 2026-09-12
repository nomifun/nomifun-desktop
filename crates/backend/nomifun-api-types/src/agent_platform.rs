//! Public API DTOs for the Agent Capability Platform control plane.
//!
//! Domain identity, digest validation, and immutable Preset/Snapshot semantics
//! remain owned by `nomifun-agent-contracts`. These DTOs are the HTTP wire
//! projection consumed by the product UI and transport adapters.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ExecutionModelRef;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentKnowledgePolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub writeback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eagerness: Option<String>,
    #[serde(default)]
    pub grounded: bool,
}

impl Default for AgentKnowledgePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            writeback: false,
            eagerness: None,
            grounded: false,
        }
    }
}

/// Immutable execution-time materialization of an AgentPreset.
///
/// Conversation, Cron, and Agent Execution persist this same consumer-neutral
/// projection. Consumer selection is not encoded as a legacy target enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentResolvedSnapshot {
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_id")]
    pub preset_id: String,
    pub preset_revision: i64,
    pub preset_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_description: Option<String>,
    #[serde(default)]
    pub instructions: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_agent_id"
    )]
    pub resolved_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_agent_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<ExecutionModelRef>,
    #[serde(default)]
    pub included_skills: Vec<String>,
    #[serde(default)]
    pub excluded_auto_skills: Vec<String>,
    #[serde(default)]
    pub initial_capabilities: Vec<String>,
    #[serde(default)]
    pub on_demand_capabilities: Vec<String>,
    #[serde(default)]
    pub required_resource_kinds: BTreeSet<String>,
    #[serde(default)]
    pub knowledge_policy: AgentKnowledgePolicy,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPresetSourceDto {
    Official,
    User,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySelectionDto {
    pub capability: ExactCatalogRefDto,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub action_allowlist: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleContractKeyDto {
    pub role_id: String,
    pub contract_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactRoleContractRefDto {
    pub key: RoleContractKeyDto,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleProviderSelectionDto {
    pub role: ExactRoleContractRefDto,
    pub provider_mount_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetDocumentDto {
    pub schema_version: String,
    pub model_route_refs: BTreeMap<String, String>,
    /// Canonical provider/model route objects. Legacy route IDs remain a
    /// separate opaque reference and are rejected at persistence time unless
    /// this map contains the matching complete record.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat_route_records: BTreeMap<String, Value>,
    pub initial_capabilities: Vec<CapabilitySelectionDto>,
    pub on_demand_capabilities: Vec<CapabilitySelectionDto>,
    pub skill_bindings: Vec<ExactCatalogRefDto>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system_role_provider_overrides: BTreeMap<String, RoleProviderSelectionDto>,
    pub persona: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starter_prompts: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetSeedDto {
    pub initial_capabilities: Vec<ExactCatalogRefDto>,
    pub on_demand_capabilities: Vec<ExactCatalogRefDto>,
    pub skill_bindings: Vec<ExactCatalogRefDto>,
    pub required_resource_kinds: BTreeSet<String>,
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
    pub required_resource_kind_count: u32,
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
    pub required_resource_kinds: BTreeSet<String>,
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
    /// User-edited configuration, including an adjusted official template.
    /// Compiled before the initial Preset/Revision is committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<AgentPresetDocumentDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentPresetFromTemplateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<AgentChatModelSelectionDto>,
    /// Prepare/reuse an internal session-only configuration for direct official
    /// Agent launch. False explicitly creates a personal Agent in the library.
    #[serde(default)]
    pub reuse_existing: bool,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub model_route_refs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat_route_records: BTreeMap<String, Value>,
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
    #[serde(default)]
    pub contribution_locks: Vec<ContributionLockDto>,
    pub created_by: String,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContributionLockDto {
    pub source_kind: String,
    pub source_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miniapp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_binding_id: Option<String>,
    pub contribution_id: String,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentContributionLifecycleDto {
    Active,
    Replaced {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_digest: Option<String>,
    },
    Disabled {
        reason: String,
    },
    Unavailable {
        code: String,
        reason: String,
    },
    MiniAppActiveReleaseChanged {
        release_id: String,
        release_digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentContributionDto {
    pub source_kind: String,
    pub source_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miniapp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_binding_id: Option<String>,
    pub contribution_id: String,
    pub contract_digest: String,
    pub lifecycle: CurrentContributionLifecycleDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContributionLifecycleImpactDto {
    Active,
    Replaced {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_digest: Option<String>,
    },
    Disabled {
        reason: String,
    },
    Unavailable {
        code: String,
        reason: String,
    },
    Uninstalled,
    MiniAppActiveReleaseChanged {
        release_id: String,
        release_digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContributionContractImpactDto {
    Exact,
    Compatible,
    Breaking {
        expected_contract_digest: String,
        actual_contract_digest: String,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionUseReadinessDto {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactRecoveryActionDto {
    Retry,
    SwitchSource,
    RestoreSource,
    ForkRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContributionImpactDto {
    pub lock: ContributionLockDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentContributionDto>,
    pub lifecycle: ContributionLifecycleImpactDto,
    pub contract: ContributionContractImpactDto,
    pub new_use: RevisionUseReadinessDto,
    pub recovery_actions: BTreeSet<ImpactRecoveryActionDto>,
    pub ignored_alternate_source_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionImpactSummaryDto {
    pub total: u32,
    pub ready: u32,
    pub blocked: u32,
    pub active_exact: u32,
    pub compatible_replace: u32,
    pub breaking_replace: u32,
    pub disabled: u32,
    pub uninstalled: u32,
    pub unavailable: u32,
    pub active_release_change_compatible: u32,
    pub active_release_change_breaking: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionImpactStatusDto {
    Ready,
    ActionRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionImpactConsumerKindDto {
    AgentBinding,
    RemoteBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionImpactConsumerDto {
    pub kind: RevisionImpactConsumerKindDto,
    pub consumer_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    pub binding_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevisionImpactResponse {
    pub preset_revision_ref: PresetRevisionRefDto,
    pub catalog_digest: String,
    pub status: RevisionImpactStatusDto,
    pub summary: RevisionImpactSummaryDto,
    pub contributions: Vec<ContributionImpactDto>,
    pub affected_consumers: Vec<RevisionImpactConsumerDto>,
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
pub struct AgentResourceSelectionDto {
    pub resource_kind: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionRequestDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<AgentChatModelSelectionDto>,
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_id")]
    pub preset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Product resource choices only. The host validates ownership and derives
    /// typed operations; clients cannot submit an AgentBinding or grant
    /// themselves resource permissions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_selections: Vec<AgentResourceSelectionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentSessionResponseDto {
    pub agent_session_id: String,
    pub agent_binding: AgentBindingValueDto,
    pub state: String,
    pub cursor: SessionCursorDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchAgentSessionPresetRequestDto {
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_id")]
    pub preset_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_selections: Vec<AgentResourceSelectionDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchAgentSessionPresetResponseDto {
    pub agent_session_id: String,
    pub agent_binding: AgentBindingValueDto,
    pub state: String,
    pub cursor: SessionCursorDto,
}

/// Product selection only; route, protocol and credential facts are host-owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChatModelSelectionDto {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use serde_json::json;

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PRESET_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    fn valid_snapshot() -> serde_json::Value {
        json!({
            "preset_id": PRESET_ID,
            "preset_revision": 1,
            "preset_name": "Coding",
            "resolved_model": {
                "provider_id": PROVIDER_ID,
                "model": "model-a"
            },
            "knowledge_policy": {
                "enabled": false,
                "writeback": false,
                "grounded": false
            },
            "initial_capabilities": [],
            "on_demand_capabilities": [],
            "required_resource_kinds": [],
            "included_skills": [],
            "excluded_auto_skills": [],
            "warnings": []
        })
    }

    #[test]
    fn snapshot_round_trips_without_a_consumer_target() {
        let snapshot: AgentResolvedSnapshot =
            serde_json::from_value(valid_snapshot()).expect("valid Agent snapshot");
        let encoded = serde_json::to_value(&snapshot).expect("serialize Agent snapshot");
        assert_eq!(encoded["preset_id"], PRESET_ID);
        assert_eq!(encoded["resolved_model"]["provider_id"], PROVIDER_ID);
        assert!(encoded.get("target").is_none());
    }

    #[test]
    fn snapshot_rejects_removed_target_and_legacy_projection_fields() {
        let legacy_override_field = ["preset_", "overrides"].concat();
        for field in ["target", "source"] {
            let mut value = valid_snapshot();
            value
                .as_object_mut()
                .expect("snapshot object")
                .insert(field.to_owned(), json!("legacy"));
            assert!(
                serde_json::from_value::<AgentResolvedSnapshot>(value).is_err(),
                "removed field {field} must fail closed"
            );
        }
        let mut value = valid_snapshot();
        value
            .as_object_mut()
            .expect("snapshot object")
            .insert(legacy_override_field, json!("legacy"));
        assert!(
            serde_json::from_value::<AgentResolvedSnapshot>(value).is_err(),
            "removed legacy override field must fail closed"
        );
    }

    #[test]
    fn snapshot_rejects_noncanonical_provider_and_agent_ids() {
        let mut invalid_provider = valid_snapshot();
        invalid_provider["resolved_model"]["provider_id"] = json!("openai");
        assert!(serde_json::from_value::<AgentResolvedSnapshot>(invalid_provider).is_err());

        let mut invalid_agent = valid_snapshot();
        invalid_agent["resolved_agent_id"] = json!("nomi");
        assert!(serde_json::from_value::<AgentResolvedSnapshot>(invalid_agent).is_err());
    }

    #[test]
    fn agent_preset_source_has_no_package_variant() {
        assert_eq!(
            serde_json::to_string(&AgentPresetSourceDto::Official).unwrap(),
            "\"official\""
        );
        assert_eq!(
            serde_json::to_string(&AgentPresetSourceDto::User).unwrap(),
            "\"user\""
        );
        assert!(serde_json::from_str::<AgentPresetSourceDto>("\"package\"").is_err());
    }

    #[test]
    fn create_agent_session_request_rejects_client_binding() {
        let request = json!({
            "preset_id": PRESET_ID,
            "title": "Session",
            "agent_binding": {
                "preset_revision_ref": {
                    "preset_id": PRESET_ID,
                    "revision": 1,
                    "revision_digest": "revision"
                },
                "resolved_snapshot_ref": {
                    "snapshot_id": "snapshot",
                    "snapshot_digest": "snapshot"
                },
                "typed_resource_bindings": [],
                "binding_version": 1
            }
        });

        assert!(
            serde_json::from_value::<CreateAgentSessionRequestDto>(request).is_err(),
            "agent_binding must remain a server-owned output"
        );
    }

    #[test]
    fn create_agent_session_request_accepts_product_resource_selections_only() {
        let request = serde_json::from_value::<CreateAgentSessionRequestDto>(json!({
            "preset_id": PRESET_ID,
            "title": "Companion Session",
            "resource_selections": [
                {"resource_kind": "companion", "resource_id": "companion-1"},
                {"resource_kind": "channel", "resource_id": "channel-1"}
            ]
        }))
        .expect("product resource selections are supported Session input");

        assert_eq!(request.resource_selections.len(), 2);
        assert_eq!(request.resource_selections[0].resource_kind, "companion");
        assert_eq!(request.resource_selections[1].resource_id, "channel-1");

        assert!(
            serde_json::from_value::<CreateAgentSessionRequestDto>(json!({
                "preset_id": PRESET_ID,
                "resource_selections": [{
                    "resource_kind": "companion",
                    "resource_id": "companion-1",
                    "operations": ["admin"]
                }]
            }))
            .is_err(),
            "clients must not be able to submit resource operations"
        );
    }

    #[test]
    fn revision_save_request_rejects_client_submitted_internal_locks() {
        let request = json!({
            "preview_digest": "preview",
            "draft": {
                "preset_id": PRESET_ID,
                "display_name": "Agent",
                "document": {
                    "schema_version": "1.0.0",
                    "model_route_refs": {},
                    "initial_capabilities": [],
                    "on_demand_capabilities": [],
                    "skill_bindings": [],
                    "resource_bindings": [],
                    "persona": "",
                    "instructions": ""
                }
            },
            "contribution_locks": []
        });
        assert!(
            serde_json::from_value::<SaveAgentPresetRevisionRequest>(request).is_err(),
            "internal ContributionLock values are server-generated and read-only"
        );
    }
}
