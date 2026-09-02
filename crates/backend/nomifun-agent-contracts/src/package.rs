//! Canonical Package, Capability, Skill, MCP, and narrow plugin-host contracts.
//!
//! This module contains metadata and wire contracts only. It intentionally does
//! not define executable plugin traits, handler objects, runtime behavior, or
//! access to application-root services.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ActionId, ArtifactEnvelope, CanonicalErrorCode, CanonicalSchemaRef, CapabilityId, DigestHex,
    ExactVersionRef, HostPortId, LogicalArtifactRef, McpServerId, McpToolKey, PackageId,
    PluginMountId, ResourceKind, RuntimeFeatureId, RuntimeTarget, ScopeKey, ServiceKeyId, SkillId,
    StateKey, StrictJsonValue, VersionString,
};

pub type PackageRef = ExactVersionRef<PackageId>;
pub type CapabilityRef = ExactVersionRef<CapabilityId>;
pub type SkillRef = ExactVersionRef<SkillId>;
pub type ServiceKeyRef = ExactVersionRef<ServiceKeyId>;
pub type HostPortRef = ExactVersionRef<HostPortId>;
pub type RuntimeFeatureRef = ExactVersionRef<RuntimeFeatureId>;

pub type PackageManifestArtifact = ArtifactEnvelope<PackageManifest>;
pub type ServiceKeyDagArtifact = ArtifactEnvelope<ServiceKeyDagPayload>;
pub type TargetPackageInventoryArtifact = ArtifactEnvelope<TargetPackageInventoryPayload>;

#[derive(
    Clone,
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
#[serde(transparent)]
pub struct ExecutionRoleId(pub String);

impl From<&str> for ExecutionRoleId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ExecutionRoleId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for ExecutionRoleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleContractKey {
    pub role_id: ExecutionRoleId,
    pub contract_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExactRoleContractRef {
    pub key: RoleContractKey,
    pub contract_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExactRoleProviderRef {
    pub role: ExactRoleContractRef,
    pub package: PackageRef,
    pub mount_id: PluginMountId,
    pub contribution_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleProviderSelection {
    pub role: ExactRoleContractRef,
    pub provider_mount_id: PluginMountId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallationRoleBinding {
    pub selection: RoleProviderSelection,
    pub binding_version: u64,
    pub updated_at_ms: i64,
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
pub enum RoleMemberRequirement {
    Required,
    Optional,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleMemberContract {
    pub capability: CapabilityRef,
    pub capability_manifest_digest: DigestHex,
    pub requirement: RoleMemberRequirement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleContractManifest {
    pub key: RoleContractKey,
    pub members: Vec<RoleMemberContract>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serialized_target_resource_kind: Option<ResourceKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleProviderMemberContribution {
    pub supported_platforms: Vec<PlatformConstraint>,
    pub required_resource_kinds: BTreeSet<ResourceKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleProviderContribution {
    pub role: ExactRoleContractRef,
    pub display: LocalizedMetadata,
    pub members: BTreeMap<CapabilityId, RoleProviderMemberContribution>,
}

pub const AGENT_CORE_PACKAGE_ID: &str = "platform.agent-core";
pub const AGENT_CORE_MOUNT_ID: &str = "platform-agent-core";
pub const AGENT_SESSION_COMMAND_SERVICE_ID: &str =
    "service.agent-session-command.v1";
pub const AGENT_SESSION_QUERY_SERVICE_ID: &str =
    "service.agent-session-query.v1";
pub const AGENT_SESSION_SERVICE_VERSION: &str = "1.0.0";

pub fn agent_core_package_ref() -> PackageRef {
    PackageRef {
        id: PackageId::from(AGENT_CORE_PACKAGE_ID),
        version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
    }
}

pub fn agent_core_mount_id() -> PluginMountId {
    PluginMountId::from(AGENT_CORE_MOUNT_ID)
}

pub fn agent_session_command_service_ref() -> ServiceKeyRef {
    ServiceKeyRef {
        id: ServiceKeyId::from(AGENT_SESSION_COMMAND_SERVICE_ID),
        version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
    }
}

pub fn agent_session_query_service_ref() -> ServiceKeyRef {
    ServiceKeyRef {
        id: ServiceKeyId::from(AGENT_SESSION_QUERY_SERVICE_ID),
        version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalizedMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub localized_names: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub localized_descriptions: BTreeMap<String, String>,
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
pub enum PluginSourceKind {
    Bundled,
    TestFixture,
    ManagedLocal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginSourceMetadata {
    pub source_kind: PluginSourceKind,
    pub source_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<DigestHex>,
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
pub enum PluginBootCriticality {
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
pub enum PluginDesiredState {
    Enabled,
    Disabled,
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
pub enum PluginEffectiveState {
    Disabled,
    Blocked,
    Failed,
    Active,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginBootState {
    pub criticality: PluginBootCriticality,
    pub desired_state: PluginDesiredState,
    pub effective_state: PluginEffectiveState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<CanonicalErrorCode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InProcessEntrypointMetadata {
    pub entrypoint_profile: String,
    pub entrypoint_id: String,
    pub contract_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub schema_version: VersionString,
    pub host_contract_version: VersionString,
    pub package_id: PackageId,
    pub package_version: VersionString,
    pub display: LocalizedMetadata,
    pub package_dependencies: Vec<PackageRef>,
    pub requires_runtime_features: Vec<RuntimeFeatureRef>,
    pub config_schema: StrictJsonValue,
    pub provides_services: Vec<ServiceProvision>,
    pub requires_services: Vec<ServiceRequirement>,
    pub entrypoint: InProcessEntrypointMetadata,
    pub contributions: PackageContributions,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageContributions {
    pub capabilities: Vec<CapabilityManifest>,
    pub skills: Vec<SkillDefinition>,
    pub mcp_tools: Vec<McpToolCapabilityMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_contracts: Vec<RoleContractManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_providers: Vec<RoleProviderContribution>,
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
pub enum CapabilityKind {
    Tool,
    ContextContributor,
    ResourceProvider,
    EventSource,
    EventConsumer,
    TurnMiddleware,
    Transport,
    Scheduler,
    BackgroundService,
    UiContribution,
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
pub enum EffectClass {
    Pure,
    ReadLocal,
    ReadSensitive,
    WriteReversible,
    WriteDurable,
    ExecuteLocal,
    ExternalTransmit,
    Destructive,
    Irreversible,
    Physical,
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
pub enum ToolPresentationKind {
    NativeCoding,
    FunctionTool,
    CodeMode,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "constraint", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlatformConstraint {
    Any,
    Targets {
        host_targets: BTreeSet<RuntimeTarget>,
        host_surfaces: BTreeSet<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityConflict {
    pub capability: CapabilityRef,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityActionDescriptor {
    pub action_id: ActionId,
    pub input_schema: CanonicalSchemaRef,
    pub output_schema: CanonicalSchemaRef,
    pub effect_class: EffectClass,
    pub presentation: ToolPresentationKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityContributions {
    pub actions: Vec<CapabilityActionDescriptor>,
    pub context_schema_refs: Vec<CanonicalSchemaRef>,
    pub event_schema_refs: Vec<CanonicalSchemaRef>,
    pub resource_kinds: BTreeSet<ResourceKind>,
    pub host_ports: Vec<HostPortRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityManifest {
    pub id: CapabilityId,
    pub version: VersionString,
    pub kind: CapabilityKind,
    pub package: PackageRef,
    pub display: LocalizedMetadata,
    pub requires: Vec<CapabilityRef>,
    pub conflicts: Vec<CapabilityConflict>,
    pub supported_surfaces: BTreeSet<String>,
    pub requires_runtime_features: Vec<RuntimeFeatureRef>,
    pub supported_platforms: Vec<PlatformConstraint>,
    pub config_schema: StrictJsonValue,
    pub contributions: CapabilityContributions,
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
pub enum SkillResourceKind {
    Reference,
    Template,
    Example,
    Script,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceRef {
    pub kind: SkillResourceKind,
    pub artifact: LogicalArtifactRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillDefinition {
    pub id: SkillId,
    pub version: VersionString,
    pub package: PackageRef,
    pub display: LocalizedMetadata,
    pub body_ref: LogicalArtifactRef,
    pub resources: Vec<SkillResourceRef>,
    pub requires_capabilities: Vec<CapabilityRef>,
    pub supported_surfaces: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpToolCapabilityMapping {
    pub package: PackageRef,
    pub server_id: McpServerId,
    pub canonical_tool_key: McpToolKey,
    pub schema_digest: DigestHex,
    pub capability: CapabilityRef,
    pub materialization_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceProvision {
    pub service: ServiceKeyRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequirement {
    pub service: ServiceKeyRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyDagNode {
    pub package: PackageRef,
    pub mount_id: PluginMountId,
    pub provides: Vec<ServiceKeyRef>,
    pub requires: Vec<ServiceKeyRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyDagEdge {
    pub service: ServiceKeyRef,
    pub provider_mount_id: PluginMountId,
    pub consumer_mount_id: PluginMountId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyDagPayload {
    pub schema_version: VersionString,
    pub nodes: Vec<ServiceKeyDagNode>,
    pub edges: Vec<ServiceKeyDagEdge>,
    pub topological_start_order: Vec<PluginMountId>,
    pub reverse_stop_order: Vec<PluginMountId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginIdentityDescriptor {
    pub package: PackageRef,
    pub mount_id: PluginMountId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidatedPluginConfig {
    pub schema_digest: DigestHex,
    pub config_revision: u64,
    pub value: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceHandleDescriptor {
    pub service: ServiceKeyRef,
    pub provider_package: PackageRef,
    pub provider_mount_id: PluginMountId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredServiceViewDescriptor {
    pub provided_services: Vec<ServiceKeyRef>,
    pub required_service_handles: Vec<ServiceHandleDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostPortBindingDescriptor {
    pub port: HostPortRef,
    pub request_schema: CanonicalSchemaRef,
    pub response_schema: CanonicalSchemaRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypedCommandPortDescriptor {
    pub port: HostPortRef,
    pub command_schema: CanonicalSchemaRef,
    pub receipt_schema: CanonicalSchemaRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainOutboxPortDescriptor {
    pub port: HostPortRef,
    pub event_schema: CanonicalSchemaRef,
    pub cursor_schema: CanonicalSchemaRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancellationDescriptor {
    pub cancellation_port: HostPortRef,
    pub scope_key: ScopeKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedTaskRegistrationDescriptor {
    pub registrar_port: HostPortRef,
    pub scope_key: ScopeKey,
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
pub enum PluginStateMethod {
    Get,
    Set,
    Delete,
    CompareAndSwap,
}

impl PluginStateMethod {
    pub const REQUIRED: [Self; 4] = [Self::Get, Self::Set, Self::Delete, Self::CompareAndSwap];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateHandleDescriptor {
    pub package_id: PackageId,
    pub mount_id: PluginMountId,
    pub methods: BTreeSet<PluginStateMethod>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateNamespace {
    pub package_id: PackageId,
    pub mount_id: PluginMountId,
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateEntry {
    pub namespace: PluginStateNamespace,
    pub revision: u64,
    pub state_format_version: VersionString,
    pub writer_package_version: VersionString,
    pub value: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateGetRequest {
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateGetResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<PluginStateEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateSetRequest {
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
    pub state_format_version: VersionString,
    pub value: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateSetResponse {
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateDeleteRequest {
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateDeleteResponse {
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginStateCompareAndSwapRequest {
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
    pub expected_revision: u64,
    pub state_format_version: VersionString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<StrictJsonValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginStateCompareAndSwapOutcome {
    Applied {
        revision: u64,
    },
    Conflict {
        current_revision: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_value: Option<StrictJsonValue>,
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
pub enum PluginRegistrarOperation {
    ProvideService,
    ContributeCapability,
    ContributeSkill,
    ContributeMcpToolMapping,
    ContributeRoleProvider,
    BindHostPort,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistrarDescriptor {
    pub identity: PluginIdentityDescriptor,
    pub allowed_operations: BTreeSet<PluginRegistrarOperation>,
    pub declared_capability_ids: BTreeSet<CapabilityId>,
    pub declared_skill_ids: BTreeSet<SkillId>,
    pub declared_mcp_tool_keys: BTreeSet<McpToolKey>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub declared_role_ids: BTreeSet<ExecutionRoleId>,
    pub declared_service_keys: BTreeSet<ServiceKeyId>,
    pub declared_host_ports: BTreeSet<HostPortId>,
}

/// The complete per-mount plugin context descriptor.
///
/// Root `PluginHost`, SQLite/DB pools, Capability or Session registries,
/// EventBus, `AppServices`, `GatewayDeps`, ambient filesystem roots, credential
/// stores, and arbitrary service locators are intentionally absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginContextDescriptor {
    pub identity: PluginIdentityDescriptor,
    pub source: PluginSourceMetadata,
    pub validated_config: ValidatedPluginConfig,
    pub state: PluginStateHandleDescriptor,
    pub declared_services: DeclaredServiceViewDescriptor,
    pub host_ports: Vec<HostPortBindingDescriptor>,
    pub typed_command_ports: Vec<TypedCommandPortDescriptor>,
    pub domain_outbox_ports: Vec<DomainOutboxPortDescriptor>,
    pub cancellation: CancellationDescriptor,
    pub managed_task_registration: ManagedTaskRegistrationDescriptor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistrationMetadata {
    pub manifest: PackageManifestArtifact,
    pub mount_id: PluginMountId,
    pub source: PluginSourceMetadata,
    pub boot_state: PluginBootState,
    pub registrar: PluginRegistrarDescriptor,
    pub context: PluginContextDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetVersionPolicy {
    pub package_version: VersionString,
    pub capability_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCapabilityContribution {
    pub capability: CapabilityRef,
    pub kind: CapabilityKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetPackageContribution {
    pub package: PackageRef,
    pub source: PluginSourceMetadata,
    pub capabilities: Vec<TargetCapabilityContribution>,
    pub skills: Vec<SkillRef>,
    pub mcp_tools: Vec<McpToolCapabilityMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_contracts: Vec<RoleContractManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_providers: Vec<RoleProviderContribution>,
}

/// Digest input for the target first-party contribution inventory.
///
/// The payload intentionally has no digest field. Use
/// [`TargetPackageInventoryArtifact`] to attach its canonical digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetPackageInventoryPayload {
    pub schema_version: VersionString,
    pub inventory_version: VersionString,
    pub version_policy: TargetVersionPolicy,
    pub packages: Vec<TargetPackageContribution>,
}
