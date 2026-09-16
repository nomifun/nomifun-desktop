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
    /// An exact, independently published capability in this Provider's Package.
    /// The Role member keeps its canonical identity; this mapping identifies
    /// the implementation without claiming that identity. None denotes a
    /// host-supplied typed export, not an implicit capability lookup/fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<CapabilityRef>,
    pub supported_platforms: Vec<PlatformConstraint>,
    // Resources of this implementation, projected into the selected member's
    // compiled policy. For mapped exports this must match the implementation
    // manifest. Private Tool/Context requirements may differ from the facade;
    // typed ResourceProvider outputs and serialized Role targets may not.
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JavaScriptEntrypointMetadata {
    pub normalized_relative_path: String,
    pub module_digest: DigestHex,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackageEntrypointMetadata {
    InProcess(InProcessEntrypointMetadata),
    #[serde(rename = "javascript")]
    JavaScript(JavaScriptEntrypointMetadata),
}

impl PackageEntrypointMetadata {
    pub fn in_process(metadata: InProcessEntrypointMetadata) -> Self {
        Self::InProcess(metadata)
    }

    pub fn javascript(metadata: JavaScriptEntrypointMetadata) -> Self {
        Self::JavaScript(metadata)
    }

    pub fn as_in_process(&self) -> Option<&InProcessEntrypointMetadata> {
        match self {
            Self::InProcess(metadata) => Some(metadata),
            Self::JavaScript(_) => None,
        }
    }

    pub fn as_in_process_mut(&mut self) -> Option<&mut InProcessEntrypointMetadata> {
        match self {
            Self::InProcess(metadata) => Some(metadata),
            Self::JavaScript(_) => None,
        }
    }

    pub fn as_javascript(&self) -> Option<&JavaScriptEntrypointMetadata> {
        match self {
            Self::InProcess(_) => None,
            Self::JavaScript(metadata) => Some(metadata),
        }
    }

    pub fn as_javascript_mut(&mut self) -> Option<&mut JavaScriptEntrypointMetadata> {
        match self {
            Self::InProcess(_) => None,
            Self::JavaScript(metadata) => Some(metadata),
        }
    }

    pub fn host_contract_version(&self) -> &VersionString {
        match self {
            Self::InProcess(metadata) => &metadata.contract_version,
            Self::JavaScript(metadata) => &metadata.host_protocol_version,
        }
    }
}

impl From<InProcessEntrypointMetadata> for PackageEntrypointMetadata {
    fn from(value: InProcessEntrypointMetadata) -> Self {
        Self::InProcess(value)
    }
}

impl From<JavaScriptEntrypointMetadata> for PackageEntrypointMetadata {
    fn from(value: JavaScriptEntrypointMetadata) -> Self {
        Self::JavaScript(value)
    }
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
    pub entrypoint: PackageEntrypointMetadata,
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

/// Whether an Agent author may select a Capability Module directly.
///
/// `CapabilityKind` remains a catalog presentation summary. It is not an
/// authorization switch and must not be used to infer Agent authorability.
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
pub enum CapabilityAuthoringPolicy {
    Direct,
    DependencyOnly,
    PlatformManaged,
    Internal,
}

impl CapabilityAuthoringPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::DependencyOnly => "dependency_only",
            Self::PlatformManaged => "platform_managed",
            Self::Internal => "internal",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "dependency_only" => Some(Self::DependencyOnly),
            "platform_managed" => Some(Self::PlatformManaged),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

/// A platform consumer that can understand and resolve a published
/// Capability contribution.
///
/// These identities are deliberately different from host execution surfaces
/// such as `desktop` and `headless`.
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
pub enum CapabilityConsumer {
    Agent,
    Gateway,
    Knowledge,
    Remote,
    Automation,
    Ui,
    #[serde(rename = "plugin_service")]
    PluginService,
}

impl CapabilityConsumer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Gateway => "gateway",
            Self::Knowledge => "knowledge",
            Self::Remote => "remote",
            Self::Automation => "automation",
            Self::Ui => "ui",
            Self::PluginService => "plugin_service",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "agent" => Some(Self::Agent),
            "gateway" => Some(Self::Gateway),
            "knowledge" => Some(Self::Knowledge),
            "remote" => Some(Self::Remote),
            "automation" => Some(Self::Automation),
            "ui" => Some(Self::Ui),
            "plugin_service" => Some(Self::PluginService),
            _ => None,
        }
    }
}

/// Consumer declarations are carried in the existing surface declaration set
/// with an explicit namespace so older host compilers can continue matching
/// plain host surfaces without confusing them with Catalog consumers.
pub const CAPABILITY_CONSUMER_SURFACE_PREFIX: &str = "consumer:";
/// Namespaced authoring declaration stored beside existing surface metadata.
/// It is filtered from host surfaces and frozen into the manifest digest.
pub const CAPABILITY_AUTHORING_SURFACE_PREFIX: &str = "authoring:";

pub fn capability_surface_declarations<H, I, J>(
    host_surfaces: I,
    supported_consumers: J,
) -> BTreeSet<String>
where
    H: AsRef<str>,
    I: IntoIterator<Item = H>,
    J: IntoIterator<Item = CapabilityConsumer>,
{
    host_surfaces
        .into_iter()
        .map(|surface| surface.as_ref().to_owned())
        .chain(supported_consumers.into_iter().map(|consumer| {
            format!(
                "{CAPABILITY_CONSUMER_SURFACE_PREFIX}{}",
                consumer.as_str()
            )
        }))
        .collect()
}

pub fn capability_module_surface_declarations<H, I, J>(
    host_surfaces: I,
    supported_consumers: J,
    authoring: CapabilityAuthoringPolicy,
) -> BTreeSet<String>
where
    H: AsRef<str>,
    I: IntoIterator<Item = H>,
    J: IntoIterator<Item = CapabilityConsumer>,
{
    let mut declarations = capability_surface_declarations(host_surfaces, supported_consumers);
    declarations.insert(format!(
        "{CAPABILITY_AUTHORING_SURFACE_PREFIX}{}",
        authoring.as_str()
    ));
    declarations
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
    /// A UI consumer contract, not an executable Tool or host DOM reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_slot: Option<UiContributionSlot>,
    pub actions: Vec<CapabilityActionDescriptor>,
    pub context_schema_refs: Vec<CanonicalSchemaRef>,
    #[serde(default, skip_serializing_if = "ContextContributionPhase::is_session_start")]
    pub context_phase: ContextContributionPhase,
    pub event_schema_refs: Vec<CanonicalSchemaRef>,
    pub resource_kinds: BTreeSet<ResourceKind>,
    pub host_ports: Vec<HostPortRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiContributionSlot {
    AgentSession,
}

/// When the selected Context consumer invokes a contribution. This is part of
/// the capability contract, not an implementation-specific scheduling hint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextContributionPhase {
    #[default]
    SessionStart,
    /// Before each main inference step for the current user turn, including
    /// follow-up steps after tools. Not a once-per-message hook.
    BeforeTurn,
}

impl ContextContributionPhase {
    pub fn is_session_start(&self) -> bool {
        *self == Self::SessionStart
    }
}

/// Host-owned turn facts; no image bytes, credentials, arbitrary host context
/// or authority fields cross this boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextTurnInput {
    pub source_message_id: String,
    pub text: String,
    pub image_media_types: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextContributionInput {
    #[default]
    SessionStart,
    BeforeTurn { turn: ContextTurnInput },
}

impl ContextContributionInput {
    pub fn phase(&self) -> ContextContributionPhase {
        match self {
            Self::SessionStart => ContextContributionPhase::SessionStart,
            Self::BeforeTurn { .. } => ContextContributionPhase::BeforeTurn,
        }
    }

    pub fn is_session_start(&self) -> bool {
        matches!(self, Self::SessionStart)
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Self::BeforeTurn { turn } = self {
            if turn.source_message_id.trim().is_empty() {
                return Err("Context turn requires a source message identity".into());
            }
            let bytes = crate::canonical_json_bytes(self).map_err(|error| error.to_string())?;
            if bytes.len() > 256 * 1024 {
                return Err("Context turn input exceeds 256 KiB".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityManifest {
    pub id: CapabilityId,
    pub contribution_id: crate::ContributionId,
    pub version: VersionString,
    pub kind: CapabilityKind,
    pub package: PackageRef,
    pub display: LocalizedMetadata,
    pub requires: Vec<CapabilityRef>,
    pub conflicts: Vec<CapabilityConflict>,
    /// Surface declarations contain plain host execution surfaces plus
    /// namespaced `consumer:<id>` Catalog consumer declarations. Callers must
    /// use [`CapabilityManifest::host_surfaces`] or
    /// [`CapabilityManifest::supported_consumers`] instead of treating the two
    /// namespaces as interchangeable.
    pub supported_surfaces: BTreeSet<String>,
    pub requires_runtime_features: Vec<RuntimeFeatureRef>,
    pub supported_platforms: Vec<PlatformConstraint>,
    pub config_schema: StrictJsonValue,
    pub contributions: CapabilityContributions,
}

impl CapabilityManifest {
    pub fn host_surfaces(&self) -> BTreeSet<&str> {
        self.supported_surfaces
            .iter()
            .map(String::as_str)
            .filter(|surface| {
                !surface.starts_with(CAPABILITY_CONSUMER_SURFACE_PREFIX)
                    && !surface.starts_with(CAPABILITY_AUTHORING_SURFACE_PREFIX)
            })
            .collect()
    }

    /// Resolve the explicit Module authoring policy. Existing manifests that
    /// predate the declaration use a narrow kind-based baseline until their
    /// owning UARC domain task republishes them as modules.
    pub fn authoring_policy(&self) -> Result<CapabilityAuthoringPolicy, String> {
        let consumers = self.supported_consumers()?;
        let declarations = self
            .supported_surfaces
            .iter()
            .filter_map(|surface| surface.strip_prefix(CAPABILITY_AUTHORING_SURFACE_PREFIX))
            .collect::<Vec<_>>();
        if declarations.len() > 1 {
            return Err(format!(
                "capability {} declares more than one authoring policy",
                self.id.as_ref()
            ));
        }
        declarations.first().map_or_else(
            || Ok(self.default_authoring_policy(&consumers)),
            |value| CapabilityAuthoringPolicy::from_str(value).ok_or_else(|| {
                format!(
                    "capability {} declares unknown authoring policy {}",
                    self.id.as_ref(),
                    value
                )
            }),
        )
    }

    fn default_authoring_policy(
        &self,
        consumers: &BTreeSet<CapabilityConsumer>,
    ) -> CapabilityAuthoringPolicy {
        match self.kind {
            CapabilityKind::Tool
            | CapabilityKind::ContextContributor
            | CapabilityKind::EventSource
                if consumers.contains(&CapabilityConsumer::Agent) =>
            {
                CapabilityAuthoringPolicy::Direct
            }
            CapabilityKind::Tool
            | CapabilityKind::ContextContributor
            | CapabilityKind::EventSource => CapabilityAuthoringPolicy::PlatformManaged,
            CapabilityKind::ResourceProvider
            | CapabilityKind::EventConsumer
            | CapabilityKind::TurnMiddleware => CapabilityAuthoringPolicy::DependencyOnly,
            CapabilityKind::Transport
            | CapabilityKind::Scheduler
            | CapabilityKind::BackgroundService
            | CapabilityKind::UiContribution => CapabilityAuthoringPolicy::PlatformManaged,
        }
    }

    pub fn declares_actions(&self) -> bool {
        !self.contributions.actions.is_empty()
    }

    pub fn contributes_context(&self) -> bool {
        !self.contributions.context_schema_refs.is_empty()
    }

    pub fn contributes_events(&self) -> bool {
        !self.contributions.event_schema_refs.is_empty()
    }

    /// Validate the capability as one Module contract. Actions, Context and
    /// Events may coexist; `kind` is deliberately not used to make those
    /// contribution sets mutually exclusive.
    pub fn validate_module_contract(&self) -> Result<(), String> {
        if crate::is_retired_extension_authoring_capability(self.id.as_ref()) {
            return Err(format!(
                "capability {} is a retired extension authoring identity",
                self.id.as_ref()
            ));
        }
        let consumers = self.supported_consumers()?;
        let authoring = self.authoring_policy()?;
        if authoring == CapabilityAuthoringPolicy::Direct {
            if !consumers.contains(&CapabilityConsumer::Agent) {
                return Err(format!(
                    "direct capability module {} must support the Agent consumer",
                    self.id.as_ref()
                ));
            }
        }

        let mut action_ids = BTreeSet::new();
        for action in &self.contributions.actions {
            if action.action_id.as_ref().trim().is_empty()
                || action.input_schema.as_ref().trim().is_empty()
                || action.output_schema.as_ref().trim().is_empty()
            {
                return Err(format!(
                    "capability {} contains an incomplete action descriptor",
                    self.id.as_ref()
                ));
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(format!(
                    "capability {} declares duplicate action {}",
                    self.id.as_ref(),
                    action.action_id.as_ref()
                ));
            }
        }
        for (field, values) in [
            ("context_schema_refs", &self.contributions.context_schema_refs),
            ("event_schema_refs", &self.contributions.event_schema_refs),
        ] {
            let mut unique = BTreeSet::new();
            for value in values {
                if value.as_ref().trim().is_empty() || !unique.insert(value) {
                    return Err(format!(
                        "capability {} contains an empty or duplicate {field}",
                        self.id.as_ref()
                    ));
                }
            }
        }
        let mut host_ports = BTreeSet::new();
        for port in &self.contributions.host_ports {
            if port.id.as_ref().trim().is_empty()
                || port.version.as_ref().trim().is_empty()
                || !host_ports.insert(port)
            {
                return Err(format!(
                    "capability {} contains an empty or duplicate host port",
                    self.id.as_ref()
                ));
            }
        }
        if !self.contributions.context_phase.is_session_start()
            && (!self.contributes_context() || !consumers.contains(&CapabilityConsumer::Agent))
        {
            return Err(format!(
                "capability {} before_turn context requires an Agent Context contribution",
                self.id.as_ref()
            ));
        }
        Ok(())
    }

    pub fn supported_consumers(&self) -> Result<BTreeSet<CapabilityConsumer>, String> {
        let mut consumers = BTreeSet::new();
        for declaration in &self.supported_surfaces {
            let Some(value) = declaration.strip_prefix(CAPABILITY_CONSUMER_SURFACE_PREFIX) else {
                continue;
            };
            let consumer = CapabilityConsumer::from_str(value).ok_or_else(|| {
                format!(
                    "capability {} declares unknown consumer surface {declaration}",
                    self.id.as_ref()
                )
            })?;
            consumers.insert(consumer);
        }
        Ok(consumers)
    }

    pub fn supports_consumer(&self, consumer: CapabilityConsumer) -> bool {
        self.supported_consumers()
            .is_ok_and(|consumers| consumers.contains(&consumer))
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        InProcessEntrypointMetadata, JavaScriptEntrypointMetadata, PackageEntrypointMetadata,
    };
    use crate::{DigestHex, VersionString};

    #[test]
    fn context_phase_preserves_legacy_digest_and_turn_input_rejects_authority_fields() {
        use super::{CapabilityContributions, ContextContributionInput, ContextContributionPhase};
        let legacy = json!({"actions": [], "context_schema_refs": [], "event_schema_refs": [], "resource_kinds": [], "host_ports": []});
        let parsed: CapabilityContributions = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(parsed.context_phase, ContextContributionPhase::SessionStart);
        assert_eq!(serde_json::to_value(parsed).unwrap(), legacy);
        let valid = json!({"phase":"before_turn", "turn": {"source_message_id":"message-1", "text":"hello", "image_media_types":["image/png"]}});
        assert!(serde_json::from_value::<ContextContributionInput>(valid.clone()).unwrap().validate().is_ok());
        let mut invalid = valid.clone();
        invalid["turn"]["principal"] = json!("owner");
        assert!(serde_json::from_value::<ContextContributionInput>(invalid).is_err());
        let mut invalid = valid;
        invalid["turn"]["source_message_id"] = json!("");
        assert!(serde_json::from_value::<ContextContributionInput>(invalid).unwrap().validate().is_err());
    }

    #[test]
    fn module_contract_allows_actions_context_and_events_without_kind_authority() {
        use std::collections::{BTreeMap, BTreeSet};

        use super::{
            CapabilityActionDescriptor, CapabilityAuthoringPolicy, CapabilityConsumer,
            CapabilityContributions, CapabilityKind, CapabilityManifest, EffectClass,
            LocalizedMetadata, PackageRef, PlatformConstraint, ToolPresentationKind,
            capability_module_surface_declarations,
        };
        use crate::{
            ActionId, CanonicalSchemaRef, CapabilityId, ContributionId, PackageId,
            ResourceKind, StrictJsonValue,
        };

        let mut module = CapabilityManifest {
            id: CapabilityId::from("workspace.files"),
            contribution_id: ContributionId::from("module:workspace.files"),
            version: VersionString::from("1.0.0"),
            kind: CapabilityKind::Tool,
            package: PackageRef {
                id: PackageId::from("nomifun.workspace"),
                version: VersionString::from("1.0.0"),
            },
            display: LocalizedMetadata {
                name: "Workspace Files".into(),
                description: "Read and update a bound workspace.".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_module_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
                CapabilityAuthoringPolicy::Direct,
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type":"object"})),
            contributions: CapabilityContributions {
                actions: ["read", "patch"]
                    .into_iter()
                    .map(|action| CapabilityActionDescriptor {
                        action_id: ActionId::from(format!("workspace.files/{action}")),
                        input_schema: CanonicalSchemaRef::from(format!("schema://workspace.files/{action}/input")),
                        output_schema: CanonicalSchemaRef::from(format!("schema://workspace.files/{action}/output")),
                        effect_class: EffectClass::ReadSensitive,
                        presentation: ToolPresentationKind::FunctionTool,
                    })
                    .collect(),
                context_schema_refs: vec![CanonicalSchemaRef::from("schema://workspace.files/context")],
                context_phase: Default::default(),
                event_schema_refs: vec![CanonicalSchemaRef::from("schema://workspace.files/changed")],
                resource_kinds: BTreeSet::from([ResourceKind::from("workspace")]),
                host_ports: Vec::new(),
                ui_slot: None,
            },
        };

        module.validate_module_contract().expect("multi-contribution module");
        assert_eq!(module.authoring_policy().unwrap(), CapabilityAuthoringPolicy::Direct);
        assert_eq!(module.host_surfaces(), BTreeSet::from(["desktop"]));

        for kind in [
            CapabilityKind::Tool,
            CapabilityKind::ContextContributor,
            CapabilityKind::ResourceProvider,
            CapabilityKind::EventSource,
            CapabilityKind::EventConsumer,
            CapabilityKind::TurnMiddleware,
            CapabilityKind::Transport,
            CapabilityKind::Scheduler,
            CapabilityKind::BackgroundService,
            CapabilityKind::UiContribution,
        ] {
            module.kind = kind;
            module
                .validate_module_contract()
                .expect("display kind must not change explicit Module authority");
        }
        for retired in crate::RETIRED_EXTENSION_AUTHORING_CAPABILITY_IDS {
            module.id = CapabilityId::from(retired);
            module.contribution_id = ContributionId::from(format!("module:{retired}"));
            module.package.id = PackageId::from("community.republisher");
            let error = module.validate_module_contract().unwrap_err();
            assert!(error.contains("retired extension authoring identity"));
        }
    }

    #[test]
    fn package_entrypoint_preserves_first_party_in_process_metadata() {
        let entrypoint: PackageEntrypointMetadata = InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: "platform.example.entrypoint".to_owned(),
            contract_version: VersionString::from("1.0.0"),
        }
        .into();

        assert_eq!(
            entrypoint
                .as_in_process()
                .expect("in-process metadata")
                .entrypoint_id,
            "platform.example.entrypoint"
        );
        assert!(entrypoint.as_javascript().is_none());
        assert_eq!(entrypoint.host_contract_version().as_ref(), "1.0.0");
    }

    #[test]
    fn package_entrypoint_serializes_javascript_as_a_tagged_variant() {
        let entrypoint = PackageEntrypointMetadata::javascript(JavaScriptEntrypointMetadata {
            normalized_relative_path: "main.mjs".to_owned(),
            module_digest: DigestHex::from("a".repeat(64)),
            host_protocol_version: VersionString::from("1.0.0"),
            sdk_contract_version: VersionString::from("1.0.0"),
        });

        assert_eq!(
            serde_json::to_value(&entrypoint).expect("serialize entrypoint"),
            json!({
                "kind": "javascript",
                "normalized_relative_path": "main.mjs",
                "module_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "host_protocol_version": "1.0.0",
                "sdk_contract_version": "1.0.0"
            })
        );
        assert_eq!(
            entrypoint
                .as_javascript()
                .expect("JavaScript metadata")
                .normalized_relative_path,
            "main.mjs"
        );
        assert!(entrypoint.as_in_process().is_none());
    }
}
