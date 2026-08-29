use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::primitives::{ServiceKeyId, VersionString};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepoPathRef {
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub symbols: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
}

impl RepoPathRef {
    fn validate(&self) -> Result<(), DeletionContractError> {
        validate_repo_relative_path(&self.path)?;
        if let (Some(start), Some(end)) = (self.line_start, self.line_end)
            && start > end
        {
            return Err(DeletionContractError::InvalidLineRange {
                path: self.path.clone(),
                start,
                end,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeletionManifestKind {
    TriadCore,
    DomainWave,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomainWave {
    TriadCore,
    Wave1ReadCapabilities,
    Wave2CodingExtensions,
    Wave3CreativeMultimodal,
    Wave4IdentityChannelsDevices,
    Wave5AutomationSupervisionRemote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalProducer {
    pub owner_id: String,
    pub target_package_keys: BTreeSet<String>,
    pub target_capability_families: BTreeSet<String>,
    pub current_producer_refs: Vec<RepoPathRef>,
    pub canonical_contract_refs: Vec<RepoPathRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectConsumerKind {
    ProductUi,
    PublicApi,
    InternalRust,
    GeneratedClient,
    CliMaintenance,
    BackgroundWorker,
    CompositionRoot,
    RuntimeDispatcher,
    TestFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectConsumer {
    pub consumer_id: String,
    pub kind: DirectConsumerKind,
    pub current_refs: Vec<RepoPathRef>,
    pub canonical_target: String,
    pub same_change_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LegacySurfaceCategory {
    Route,
    Dto,
    EventNameFieldProjection,
    TableRepositoryMapping,
    ConfigEnvFeature,
    ModeApprovalPermission,
    FactoryManagerWiring,
    GatewayDepsWiring,
    AppServicesWiring,
    NomiRuntimeWiring,
    ConversationIdentity,
    UiRouteState,
    GeneratedContract,
    TestFixtureGolden,
    CratePackageDependency,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LegacyDisposition {
    ReplaceThenDelete,
    DeleteWithoutReplacement,
    RetainHistoricalSourceExcludedFromV4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LegacySurface {
    pub surface_id: String,
    pub category: LegacySurfaceCategory,
    pub symbols_or_patterns: BTreeSet<String>,
    pub current_refs: Vec<RepoPathRef>,
    pub disposition: LegacyDisposition,
    pub removal_boundary: String,
    pub replacement_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductionRootKind {
    ProductUiRoute,
    PublicApiRouter,
    CliMaintenance,
    BackgroundSchedulerIngress,
    CompositionStartup,
    SessionResolver,
    RuntimeDispatcher,
    PluginInventory,
    GeneratedContract,
    ReleaseArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductionRoot {
    pub root_id: String,
    pub kind: ProductionRootKind,
    pub current_refs: Vec<RepoPathRef>,
    pub expected_canonical_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllowedResidualPolicyKind {
    Empty,
    D004ExactDecrementingAllowlist,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllowedResidualEntry {
    pub residual_id: String,
    pub exact_refs: Vec<RepoPathRef>,
    pub constraints: BTreeSet<String>,
    pub allowed_until_boundary: String,
    pub target_zero_boundary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllowedResiduals {
    pub policy: AllowedResidualPolicyKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<AllowedResidualEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZeroDimension {
    SourceSymbols,
    Routes,
    Dtos,
    EventVocabulary,
    TableMappings,
    ConfigFields,
    ModeApprovalBranches,
    FactoryManagerWiring,
    GatewayDepsWiring,
    AppServicesWiring,
    TestsFixtures,
    Dependencies,
    RuntimeReachability,
    BuildArtifacts,
    D027OutstandingSet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetZeroAssertion {
    pub assertion_id: String,
    pub dimension: ZeroDimension,
    pub expected_count: u64,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourceScan,
    SchemaRouteDiff,
    CargoMetadata,
    BuildArtifactScan,
    RuntimeReachability,
    TargetedTest,
    FaultTest,
    NativeEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub gate_name: String,
    pub source_refs: Vec<RepoPathRef>,
    pub required_for_closure: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum D027Applicability {
    NotApplicable,
    DomainWave,
    FinalGlobal,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutstandingSetKind {
    OpeningSessions,
    ReadySessions,
    RunningSessions,
    UnacknowledgedRuntimeActions,
    ModelRequests,
    ToolDispatches,
    EffectDispatches,
    Tasks,
    DescendantProcesses,
    Leases,
    ResourceHandles,
    PrivateWrites,
    RuntimeReachability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutstandingSetRef {
    pub kind: OutstandingSetKind,
    pub current_refs: Vec<RepoPathRef>,
    pub expected_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D027DrainContract {
    pub applicability: D027Applicability,
    pub admission_fence_refs: Vec<RepoPathRef>,
    pub deadline_authority_refs: Vec<RepoPathRef>,
    pub outstanding_set_refs: Vec<OutstandingSetRef>,
    pub uncertain_handoff_owner: String,
    pub delete_contract_ref: String,
    pub zero_required_before_legacy_delete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClosureState {
    InventoryComplete,
    ReadyForImplementation,
    InProgress,
    Closed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClosureStatus {
    pub state: ClosureState,
    pub canonical_producer_ready: bool,
    pub direct_consumers_switched: bool,
    pub legacy_surfaces_deleted: bool,
    pub residual_zero_proven: bool,
    pub reachability_zero_proven: bool,
    pub d027_zero_proven: bool,
    pub evidence_complete: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletionManifest {
    pub schema_version: VersionString,
    pub manifest_id: String,
    pub manifest_kind: DeletionManifestKind,
    pub wave: DomainWave,
    pub workstream: String,
    pub base_sha: String,
    pub canonical_producer: CanonicalProducer,
    pub direct_consumers: Vec<DirectConsumer>,
    pub legacy_surfaces: Vec<LegacySurface>,
    pub production_roots: Vec<ProductionRoot>,
    pub allowed_residuals: AllowedResiduals,
    pub target_zero: Vec<TargetZeroAssertion>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub d027: D027DrainContract,
    pub closure_status: ClosureStatus,
}

impl DeletionManifest {
    pub fn validate(&self) -> Result<(), DeletionContractError> {
        if self.direct_consumers.is_empty() {
            return Err(DeletionContractError::EmptyCollection("direct_consumers"));
        }
        if self.legacy_surfaces.is_empty() {
            return Err(DeletionContractError::EmptyCollection("legacy_surfaces"));
        }
        if self.production_roots.is_empty() {
            return Err(DeletionContractError::EmptyCollection("production_roots"));
        }
        if self.target_zero.is_empty() {
            return Err(DeletionContractError::EmptyCollection("target_zero"));
        }
        if self
            .target_zero
            .iter()
            .any(|assertion| assertion.expected_count != 0)
        {
            return Err(DeletionContractError::NonZeroTarget);
        }
        match self.allowed_residuals.policy {
            AllowedResidualPolicyKind::Empty if !self.allowed_residuals.entries.is_empty() => {
                return Err(DeletionContractError::OrdinaryResidualAllowlistNotEmpty);
            }
            AllowedResidualPolicyKind::D004ExactDecrementingAllowlist
                if self.allowed_residuals.entries.is_empty() =>
            {
                return Err(DeletionContractError::D004ResidualAllowlistEmpty);
            }
            _ => {}
        }
        if self.d027.applicability != D027Applicability::NotApplicable
            && self
                .d027
                .outstanding_set_refs
                .iter()
                .any(|reference| reference.expected_count != 0)
        {
            return Err(DeletionContractError::NonZeroD027Target);
        }
        self.visit_repo_refs(RepoPathRef::validate)
    }

    fn visit_repo_refs(
        &self,
        mut visitor: impl FnMut(&RepoPathRef) -> Result<(), DeletionContractError>,
    ) -> Result<(), DeletionContractError> {
        for reference in &self.canonical_producer.current_producer_refs {
            visitor(reference)?;
        }
        for reference in &self.canonical_producer.canonical_contract_refs {
            visitor(reference)?;
        }
        for consumer in &self.direct_consumers {
            for reference in &consumer.current_refs {
                visitor(reference)?;
            }
        }
        for surface in &self.legacy_surfaces {
            for reference in &surface.current_refs {
                visitor(reference)?;
            }
        }
        for root in &self.production_roots {
            for reference in &root.current_refs {
                visitor(reference)?;
            }
        }
        for entry in &self.allowed_residuals.entries {
            for reference in &entry.exact_refs {
                visitor(reference)?;
            }
        }
        for evidence in &self.evidence_refs {
            for reference in &evidence.source_refs {
                visitor(reference)?;
            }
        }
        for reference in &self.d027.admission_fence_refs {
            visitor(reference)?;
        }
        for reference in &self.d027.deadline_authority_refs {
            visitor(reference)?;
        }
        for outstanding in &self.d027.outstanding_set_refs {
            for reference in &outstanding.current_refs {
                visitor(reference)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompositionRootKind {
    AppServices,
    GatewayDeps,
    AgentFactoryDeps,
    NomiBuildExtra,
    ConversationService,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompositionRootInventory {
    pub root_id: String,
    pub kind: CompositionRootKind,
    pub type_name: String,
    pub declaration_refs: Vec<RepoPathRef>,
    pub responsibility_groups: BTreeSet<String>,
    pub direct_consumer_refs: Vec<RepoPathRef>,
    pub target_disposition: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompositionInventoryPayload {
    pub schema_version: VersionString,
    pub inventory_id: String,
    pub base_sha: String,
    pub roots: Vec<CompositionRootInventory>,
    pub absent_target_types: BTreeSet<String>,
    pub service_key_target_map_ref: String,
    pub event_outbox_classification_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyTargetNode {
    pub service_key: ServiceKeyId,
    pub provider_package_key: String,
    pub purpose: String,
    pub current_provider_refs: Vec<RepoPathRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyTargetEdge {
    pub consumer_package_key: String,
    pub requires_service_key: ServiceKeyId,
    pub migration_wave: DomainWave,
    pub current_coupling_refs: Vec<RepoPathRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceKeyTargetMapPayload {
    pub schema_version: VersionString,
    pub inventory_id: String,
    pub base_sha: String,
    pub invariants: BTreeSet<String>,
    pub nodes: Vec<ServiceKeyTargetNode>,
    pub edges: Vec<ServiceKeyTargetEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventDeliveryClass {
    BestEffortProjectionWakeup,
    ReliableDomainFactTransactionalOutbox,
    DurableReceiptAuthority,
    TypedCommand,
    RequiresReliabilityDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventOutboxClassificationEntry {
    pub entry_id: String,
    pub domain: String,
    pub current_event_names: BTreeSet<String>,
    pub current_refs: Vec<RepoPathRef>,
    pub current_delivery: String,
    pub target_class: EventDeliveryClass,
    pub canonical_owner: String,
    pub requires_domain_outbox: bool,
    pub target_action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventOutboxClassificationPayload {
    pub schema_version: VersionString,
    pub inventory_id: String,
    pub base_sha: String,
    pub invariants: BTreeSet<String>,
    pub entries: Vec<EventOutboxClassificationEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DeletionContractError {
    #[error("{0} must not be empty")]
    EmptyCollection(&'static str),
    #[error("ordinary deletion manifests must have an empty allowed residual list")]
    OrdinaryResidualAllowlistNotEmpty,
    #[error("the D-004 residual policy requires an exact decrementing allowlist")]
    D004ResidualAllowlistEmpty,
    #[error("every target-zero assertion must expect count zero")]
    NonZeroTarget,
    #[error("every D-027 outstanding-set target must expect count zero")]
    NonZeroD027Target,
    #[error("path is not normalized repository-relative: {0}")]
    InvalidRepoRelativePath(String),
    #[error("invalid line range for {path}: {start}..{end}")]
    InvalidLineRange {
        path: String,
        start: u32,
        end: u32,
    },
}

fn validate_repo_relative_path(path: &str) -> Result<(), DeletionContractError> {
    let has_forbidden_segment = path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    let looks_absolute = path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains("://")
        || path
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':');
    if path.is_empty() || has_forbidden_segment || looks_absolute {
        return Err(DeletionContractError::InvalidRepoRelativePath(
            path.to_owned(),
        ));
    }
    Ok(())
}
