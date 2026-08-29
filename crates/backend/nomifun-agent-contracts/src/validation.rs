use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    digest_payload, ArtifactEnvelope, CanonicalDigestError, CanonicalErrorCode, DigestHex,
    LogicalArtifactRef, RuntimeTarget, VersionString,
};

macro_rules! validation_string_newtype {
    ($name:ident) => {
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

validation_string_newtype!(CandidateSourceSha);
validation_string_newtype!(GateCheckId);
validation_string_newtype!(HandoffId);
validation_string_newtype!(PlatformVerificationPointId);
validation_string_newtype!(ValidationRunId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ConfirmedDecisionContractDigestRef(pub DigestHex);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PlatformValidationManifestDigestRef(pub DigestHex);

/// Typed reference only. Runtime release fields remain owned by the runtime
/// contract and are not duplicated in this module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CodexRuntimeReleaseDigestRef(pub DigestHex);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCohortTuple {
    pub candidate_source_sha: CandidateSourceSha,
    pub confirmed_decision_contract_digest: ConfirmedDecisionContractDigestRef,
    pub platform_validation_manifest_digest: PlatformValidationManifestDigestRef,
    pub runtime_release_digest: CodexRuntimeReleaseDigestRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImmutableValidationInputRefs {
    pub runtime_release_manifest: LogicalArtifactRef,
    pub platform_validation_manifest: LogicalArtifactRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D025FixtureEnvelopeReference {
    pub fixture_envelope: LogicalArtifactRef,
    pub compatible_exact_case_id: String,
    pub executor_unavailable_case_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D025FixtureCaseReference {
    pub case_id: String,
    pub expected_result_variant: String,
}

/// Reference-only payload for the Runtime-owned D-025 contract. The input and
/// result types remain defined in `crate::runtime`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D025FixtureContractReferencePayload {
    pub schema_version: VersionString,
    pub input_contract_type: String,
    pub result_contract_type: String,
    pub checkpoint_mismatch_fixture: LogicalArtifactRef,
    pub required_cases: Vec<D025FixtureCaseReference>,
}

pub type D025FixtureContractReference =
    ArtifactEnvelope<D025FixtureContractReferencePayload>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionFixtureEnvelopeReferences {
    pub d025_snapshot_compatibility: D025FixtureEnvelopeReference,
    pub d026_request_admission_ordering: LogicalArtifactRef,
    pub d026_validation_outcomes: LogicalArtifactRef,
    pub d027_terminal_drain: LogicalArtifactRef,
    pub d028_platform_matrix: LogicalArtifactRef,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TargetCellId {
    WindowsDesktopX64,
    MacosDesktopArm64,
    MacosDesktopX64,
    LinuxDesktopX64,
    LinuxHeadlessX64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HostOperatingSystem {
    Windows,
    Macos,
    Linux,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HostArchitecture {
    X86_64,
    Aarch64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HostSurface {
    Desktop,
    Headless,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityContractSubject {
    CodingCodexNative,
    Browser,
    Computer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityAvailability {
    RequiredExactSet,
    ReleaseManifestDefined,
    IndependentPartialOrExactUnavailable,
    ExactUnavailable { error_code: CanonicalErrorCode },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCell {
    pub host_os: HostOperatingSystem,
    pub host_arch: HostArchitecture,
    pub host_target: RuntimeTarget,
    pub runtime_target: RuntimeTarget,
    pub host_surface: HostSurface,
    pub package_format: String,
    pub capability_availability: BTreeMap<CapabilityContractSubject, CapabilityAvailability>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedLocalTarget {
    WindowsArm64,
    LinuxArm64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOnlySurface {
    Mobile,
    WebBrowserClient,
    RobotFirmware,
    ImClient,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ValidationBoundary {
    #[serde(rename = "windows-c1-c7-continuous")]
    WindowsC1C7Continuous,
    #[serde(rename = "c8-win-pre")]
    C8WinPre,
    #[serde(rename = "hp-1")]
    Hp1,
    #[serde(rename = "c8-ma")]
    C8Ma,
    #[serde(rename = "hp-2")]
    Hp2,
    #[serde(rename = "c8-mx")]
    C8Mx,
    #[serde(rename = "c8-ld")]
    C8Ld,
    #[serde(rename = "c8-lh")]
    C8Lh,
    #[serde(rename = "merge-c8-whole-batch-fixes")]
    MergeC8WholeBatchFixes,
    #[serde(rename = "c8-merge")]
    C8Merge,
    #[serde(rename = "d027-final-drain-exact-zero")]
    D027FinalDrainExactZero,
    #[serde(rename = "c9-nomi-hard-delete")]
    C9NomiHardDelete,
    #[serde(rename = "c10-win")]
    C10Win,
    #[serde(rename = "c10-ma")]
    C10Ma,
    #[serde(rename = "c10-mx")]
    C10Mx,
    #[serde(rename = "c10-ld")]
    C10Ld,
    #[serde(rename = "c10-lh")]
    C10Lh,
    #[serde(rename = "merge-c10-whole-batch-fixes")]
    MergeC10WholeBatchFixes,
    #[serde(rename = "c10-merge")]
    C10Merge,
    #[serde(rename = "c11-same-digest-stable")]
    C11SameDigestStable,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RecheckFamily {
    C8,
    C10,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "stage_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlatformRelayStage {
    Boundary {
        boundary: ValidationBoundary,
    },
    ParallelBoundaries {
        boundaries: BTreeSet<ValidationBoundary>,
    },
    RepeatableWholeCohortRecheck {
        family: RecheckFamily,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WholeCohortRecheckPolicy {
    pub only_after_complete_round_returns: bool,
    pub merge_fixes_before_freezing_new_tuple: bool,
    pub affected_cells_run_full_gate: bool,
    pub unaffected_cells_run_native_scoped_attestation: bool,
    pub central_owner_cannot_attest_for_native_host: bool,
    pub single_fix_platform_handoff_forbidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D028PlatformMatrix {
    pub target_cells: BTreeMap<TargetCellId, TargetCell>,
    pub unsupported_local_targets: BTreeSet<UnsupportedLocalTarget>,
    pub remote_only_surfaces: BTreeSet<RemoteOnlySurface>,
    pub relay_order: Vec<PlatformRelayStage>,
    pub recheck_policy: WholeCohortRecheckPolicy,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RequiredEvidenceKind {
    Native,
    Informational,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredGateCheck {
    pub check_id: GateCheckId,
    pub target_cells: BTreeSet<TargetCellId>,
    pub command: String,
    pub required_execution_kind: RequiredEvidenceKind,
}

/// Immutable verification-point definition. Runtime state and evidence live in
/// post-run ledger records, never in the pre-run manifest payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformVerificationPoint {
    pub verification_point_id: PlatformVerificationPointId,
    pub owning_module: String,
    pub target_cell: TargetCellId,
    pub behavior_to_observe: String,
    pub exact_check_id: GateCheckId,
}

/// Pre-run immutable input. Its own digest, status, evidence, logs, and summary
/// are structurally absent and therefore cannot participate in its digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformValidationManifestPayload {
    pub manifest_version: VersionString,
    pub candidate_source_sha: CandidateSourceSha,
    pub confirmed_decision_contract_digest: ConfirmedDecisionContractDigestRef,
    pub runtime_release_digest: CodexRuntimeReleaseDigestRef,
    pub canonical_schema_manifest_digest: DigestHex,
    pub cargo_lock_digest: DigestHex,
    pub official_preset_seed_manifest_digest: DigestHex,
    pub capability_availability_manifest_digest: DigestHex,
    pub coding_codex_native_contract_digest: DigestHex,
    pub platform_matrix: D028PlatformMatrix,
    pub required_checks: Vec<RequiredGateCheck>,
    pub platform_verification_points: Vec<PlatformVerificationPoint>,
    pub decision_fixture_refs: DecisionFixtureEnvelopeReferences,
}

pub type PlatformValidationManifest = ArtifactEnvelope<PlatformValidationManifestPayload>;
pub type PlatformValidationManifestArtifact = PlatformValidationManifest;

impl PlatformValidationManifestPayload {
    pub fn payload_digest(&self) -> Result<DigestHex, CanonicalDigestError> {
        digest_payload(self)
    }
}


#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum D026OrderingCaseKind {
    RequestAdmissionCommittedBeforeFence,
    FenceCommittedBeforeOldCredentialAdmission,
    ReplacementCredentialAfterFence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum D026AdmissionOutcome {
    ContinuePreviouslyAdmittedOperationToFiniteBoundary,
    RejectRemoteAuthRequiredBeforeBindingOrSessionLookup,
    ContinueExistingSessionForSameOwnerWithExplicitSessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D026OrderingOutcome {
    pub case_kind: D026OrderingCaseKind,
    pub outcome: D026AdmissionOutcome,
    pub expected_error_code: Option<CanonicalErrorCode>,
    pub existing_session_mutated: bool,
    pub existing_binding_mutated: bool,
    pub cascade_cancelled: bool,
    pub explicit_agent_session_id_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D026OrderingOutcomeMatrix {
    pub schema_version: VersionString,
    pub outcomes: Vec<D026OrderingOutcome>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum D027DrainCaseKind {
    NoDurableAcceptedOperation,
    DurableAcceptedOperation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum D027DeadlineRule {
    Immediate,
    MinimumOfOperationAndAllAncestorExistingFiniteDeadlines,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum D027TerminalStep {
    #[serde(rename = "stop-nomi-admission")]
    StopNomiAdmission,
    #[serde(rename = "wait-existing-deadline-minimum")]
    WaitExistingDeadlineMinimum,
    #[serde(rename = "cancel")]
    Cancel,
    #[serde(rename = "dispose-runtime")]
    DisposeRuntime,
    #[serde(rename = "kill-descendants")]
    KillDescendants,
    #[serde(rename = "durable-uncertain-handoff")]
    DurableUncertainHandoff,
    #[serde(rename = "prove-outstanding-exact-zero")]
    ProveOutstandingExactZero,
    #[serde(rename = "d024-delete-agent-session")]
    D024DeleteAgentSession,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D027OutstandingSet {
    pub opening_sessions: u64,
    pub ready_sessions: u64,
    pub running_sessions: u64,
    pub unacknowledged_runtime_actions: u64,
    pub active_turns: u64,
    pub model_requests: u64,
    pub tool_dispatches: u64,
    pub effect_dispatches: u64,
    pub tasks: u64,
    pub descendant_processes: u64,
    pub leases: u64,
    pub resource_handles: u64,
    pub private_writes: u64,
    pub fallback_paths: u64,
    pub runtime_reachability: u64,
}

impl D027OutstandingSet {
    pub fn is_exact_zero(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D027TerminalSequence {
    pub case_kind: D027DrainCaseKind,
    pub deadline_rule: D027DeadlineRule,
    pub steps: Vec<D027TerminalStep>,
    pub handoff_waits_for_reconcile: bool,
    pub same_session_runtime_switch_allowed: bool,
    pub configurable_drain_timeout_allowed: bool,
    pub outstanding_after: D027OutstandingSet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D027TerminalSequenceMatrix {
    pub schema_version: VersionString,
    pub sequences: Vec<D027TerminalSequence>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CohortTupleComponent {
    CandidateSourceSha,
    ConfirmedDecisionContractDigest,
    PlatformValidationManifestDigest,
    RuntimeReleaseDigest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactAssessment {
    pub invalidating_fix_sha: CandidateSourceSha,
    pub affected_cell_ids: BTreeSet<TargetCellId>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInvalidationPlan {
    pub previous_tuple: CanonicalCohortTuple,
    pub next_tuple: CanonicalCohortTuple,
    pub changed_components: BTreeSet<CohortTupleComponent>,
    pub invalidating_fix_sha: Option<CandidateSourceSha>,
    pub stale_cell_ids: BTreeSet<TargetCellId>,
    pub full_revalidation_cell_ids: BTreeSet<TargetCellId>,
    pub native_scoped_attestation_cell_ids: BTreeSet<TargetCellId>,
    pub reusable_exact_tuple_pass_cell_ids: BTreeSet<TargetCellId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInvalidationPlanMatrix {
    pub schema_version: VersionString,
    pub plans: Vec<EvidenceInvalidationPlan>,
}

impl CanonicalCohortTuple {
    pub fn changed_components(&self, next: &Self) -> BTreeSet<CohortTupleComponent> {
        let mut changed = BTreeSet::new();
        if self.candidate_source_sha != next.candidate_source_sha {
            changed.insert(CohortTupleComponent::CandidateSourceSha);
        }
        if self.confirmed_decision_contract_digest != next.confirmed_decision_contract_digest {
            changed.insert(CohortTupleComponent::ConfirmedDecisionContractDigest);
        }
        if self.platform_validation_manifest_digest != next.platform_validation_manifest_digest {
            changed.insert(CohortTupleComponent::PlatformValidationManifestDigest);
        }
        if self.runtime_release_digest != next.runtime_release_digest {
            changed.insert(CohortTupleComponent::RuntimeReleaseDigest);
        }
        changed
    }

    pub fn plan_invalidation(
        &self,
        next: &Self,
        impact: Option<&ImpactAssessment>,
    ) -> Result<EvidenceInvalidationPlan, ValidationContractError> {
        let all_cells = required_target_cell_ids();
        let changed_components = self.changed_components(next);
        if changed_components.is_empty() {
            return Ok(EvidenceInvalidationPlan {
                previous_tuple: self.clone(),
                next_tuple: next.clone(),
                changed_components,
                invalidating_fix_sha: None,
                stale_cell_ids: BTreeSet::new(),
                full_revalidation_cell_ids: BTreeSet::new(),
                native_scoped_attestation_cell_ids: BTreeSet::new(),
                reusable_exact_tuple_pass_cell_ids: all_cells,
            });
        }

        let invalidates_all = changed_components.iter().any(|component| {
            matches!(
                component,
                CohortTupleComponent::ConfirmedDecisionContractDigest
                    | CohortTupleComponent::PlatformValidationManifestDigest
                    | CohortTupleComponent::RuntimeReleaseDigest
            )
        });

        if invalidates_all {
            return Ok(EvidenceInvalidationPlan {
                previous_tuple: self.clone(),
                next_tuple: next.clone(),
                changed_components,
                invalidating_fix_sha: impact.map(|impact| impact.invalidating_fix_sha.clone()),
                stale_cell_ids: all_cells.clone(),
                full_revalidation_cell_ids: all_cells,
                native_scoped_attestation_cell_ids: BTreeSet::new(),
                reusable_exact_tuple_pass_cell_ids: BTreeSet::new(),
            });
        }

        let impact = impact.ok_or(ValidationContractError::SourceImpactRequired)?;
        if impact.affected_cell_ids.is_empty() {
            return Err(ValidationContractError::AffectedCellsRequired);
        }
        if !impact.affected_cell_ids.is_subset(&all_cells) {
            return Err(ValidationContractError::UnknownAffectedCell);
        }
        let scoped = all_cells
            .difference(&impact.affected_cell_ids)
            .copied()
            .collect();
        Ok(EvidenceInvalidationPlan {
            previous_tuple: self.clone(),
            next_tuple: next.clone(),
            changed_components,
            invalidating_fix_sha: Some(impact.invalidating_fix_sha.clone()),
            stale_cell_ids: impact.affected_cell_ids.clone(),
            full_revalidation_cell_ids: impact.affected_cell_ids.clone(),
            native_scoped_attestation_cell_ids: scoped,
            reusable_exact_tuple_pass_cell_ids: BTreeSet::new(),
        })
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ValidationLedgerStatus {
    PendingNativeVerification,
    Pass,
    Fail,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeHostFingerprint {
    pub host_os: HostOperatingSystem,
    pub host_arch: HostArchitecture,
    pub host_target: RuntimeTarget,
    pub runtime_target: RuntimeTarget,
    pub toolchain_fingerprint_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutedGateCheck {
    pub check_id: GateCheckId,
    pub command: String,
    pub evidence_kind: RequiredEvidenceKind,
    pub exit_code: i32,
    pub output_artifact: LogicalArtifactRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationPointResult {
    pub verification_point_id: PlatformVerificationPointId,
    pub status: ValidationLedgerStatus,
    pub evidence_ref: Option<LogicalArtifactRef>,
}

/// Post-run cell result. It references immutable inputs and external evidence
/// bundles but never embeds or mutates either pre-run payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformCellEvidence {
    pub schema_version: VersionString,
    pub run_id: ValidationRunId,
    pub cell_id: TargetCellId,
    pub native_host_fingerprint: NativeHostFingerprint,
    pub cohort_tuple: CanonicalCohortTuple,
    pub input_manifest_refs: ImmutableValidationInputRefs,
    pub status: ValidationLedgerStatus,
    pub artifact_digests: BTreeMap<String, DigestHex>,
    pub coding_codex_native_exact_set_digest: DigestHex,
    pub coding_codex_native_result: ValidationLedgerStatus,
    pub capability_availability_manifest_digest: DigestHex,
    pub gate_suite_digest: DigestHex,
    pub executed_checks: Vec<ExecutedGateCheck>,
    pub unexecuted_check_reasons: BTreeMap<GateCheckId, String>,
    pub verification_point_results: Vec<VerificationPointResult>,
    pub evidence_bundle: LogicalArtifactRef,
    pub invalidation: Option<EvidenceInvalidationRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInvalidationRecord {
    pub invalidating_fix_sha: CandidateSourceSha,
    pub affected_cell_ids: BTreeSet<TargetCellId>,
    pub superseded_run_id: ValidationRunId,
    pub native_revalidation_evidence: Option<LogicalArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformValidationLedgerEntry {
    pub run_id: ValidationRunId,
    pub cell_id: TargetCellId,
    pub cohort_tuple: CanonicalCohortTuple,
    pub status: ValidationLedgerStatus,
    pub evidence_ref: Option<LogicalArtifactRef>,
    pub fix_commit: Option<CandidateSourceSha>,
    pub affected_cell_ids: BTreeSet<TargetCellId>,
    pub superseded_run_id: Option<ValidationRunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformValidationLedgerMatrix {
    pub schema_version: VersionString,
    pub entries: Vec<PlatformValidationLedgerEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellEvidenceReference {
    pub status: ValidationLedgerStatus,
    pub cohort_tuple: CanonicalCohortTuple,
    pub evidence: LogicalArtifactRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceStatusCounts {
    pub pending_native_verification: u64,
    pub pass: u64,
    pub fail: u64,
    pub stale: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSummaryFamily {
    C8,
    C10,
}

/// Post-run merge summary. Its digest belongs to an external envelope and does
/// not feed back into either immutable input digest or the current tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSummary {
    pub schema_version: VersionString,
    pub family: EvidenceSummaryFamily,
    pub cohort_tuple: CanonicalCohortTuple,
    pub input_manifest_refs: ImmutableValidationInputRefs,
    pub cell_evidence: BTreeMap<TargetCellId, CellEvidenceReference>,
    pub status_counts: EvidenceStatusCounts,
    pub closed_verification_point_ids: BTreeSet<PlatformVerificationPointId>,
    pub all_verification_points_closed: bool,
    pub global_residual_reachability_zero: bool,
    pub d027_terminal_evidence: Option<LogicalArtifactRef>,
    pub release_evidence_envelope: Option<LogicalArtifactRef>,
}

pub type EvidenceSummaryArtifact = ArtifactEnvelope<EvidenceSummary>;

impl EvidenceSummary {
    pub fn computed_status_counts(&self) -> EvidenceStatusCounts {
        let mut counts = EvidenceStatusCounts::default();
        for evidence in self.cell_evidence.values() {
            match evidence.status {
                ValidationLedgerStatus::PendingNativeVerification => {
                    counts.pending_native_verification += 1;
                }
                ValidationLedgerStatus::Pass => counts.pass += 1,
                ValidationLedgerStatus::Fail => counts.fail += 1,
                ValidationLedgerStatus::Stale => counts.stale += 1,
            }
        }
        counts
    }

    pub fn is_merge_ready(&self) -> bool {
        self.cell_evidence.keys().copied().collect::<BTreeSet<_>>() == required_target_cell_ids()
            && self.cell_evidence.values().all(|evidence| {
                evidence.status == ValidationLedgerStatus::Pass
                    && evidence.cohort_tuple == self.cohort_tuple
            })
            && self.status_counts == self.computed_status_counts()
            && self.status_counts.pending_native_verification == 0
            && self.status_counts.fail == 0
            && self.status_counts.stale == 0
            && self.all_verification_points_closed
            && self.global_residual_reachability_zero
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum HandoffBoundary {
    #[serde(rename = "hp-1")]
    Hp1,
    #[serde(rename = "hp-2")]
    Hp2,
    #[serde(rename = "c8-recheck-n")]
    C8RecheckN,
    #[serde(rename = "c10-recheck-n")]
    C10RecheckN,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCheckpoint {
    pub branch: String,
    pub shared_ref: String,
    pub local_head: CandidateSourceSha,
    pub verified_remote_sha: CandidateSourceSha,
    pub clean_worktree: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowsRecheckRequirement {
    NotApplicable,
    ReuseExactTuplePass,
    FullGate,
    NativeScopedAttestation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeValidationCommand {
    pub target_cell: TargetCellId,
    pub working_directory: String,
    pub command: String,
}

/// Cross-machine engineering handoff. Paths are repository-relative logical
/// paths; no dirty worktree, machine absolute path, or product state is valid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformHandoffBundle {
    pub schema_version: VersionString,
    pub handoff_id: HandoffId,
    pub boundary: HandoffBoundary,
    pub source_checkpoint: SourceCheckpoint,
    pub cohort_tuple: CanonicalCohortTuple,
    pub input_manifest_refs: ImmutableValidationInputRefs,
    pub target_cells: BTreeSet<TargetCellId>,
    pub windows_recheck_requirement: WindowsRecheckRequirement,
    pub prerequisites: Vec<String>,
    pub native_commands: Vec<NativeValidationCommand>,
    pub pending_verification_point_ids: BTreeSet<PlatformVerificationPointId>,
    pub compact_evidence_summary: LogicalArtifactRef,
    pub artifact_digests: BTreeMap<String, DigestHex>,
    pub return_ledger_relative_path: String,
}

pub type PlatformHandoffBundleArtifact = ArtifactEnvelope<PlatformHandoffBundle>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffBundleMatrix {
    pub schema_version: VersionString,
    pub bundles: Vec<PlatformHandoffBundle>,
}

impl PlatformHandoffBundle {
    pub fn validate(&self) -> Result<(), ValidationContractError> {
        if !self.source_checkpoint.clean_worktree {
            return Err(ValidationContractError::DirtyHandoffCheckpoint);
        }
        if self.source_checkpoint.local_head != self.source_checkpoint.verified_remote_sha
            || self.source_checkpoint.local_head != self.cohort_tuple.candidate_source_sha
        {
            return Err(ValidationContractError::HandoffShaMismatch);
        }
        if !self.target_cells.is_subset(&required_target_cell_ids()) {
            return Err(ValidationContractError::UnknownHandoffTargetCell);
        }
        let hp2_base = BTreeSet::from([
            TargetCellId::MacosDesktopX64,
            TargetCellId::LinuxDesktopX64,
            TargetCellId::LinuxHeadlessX64,
        ]);
        let hp2_with_windows = hp2_base
            .iter()
            .copied()
            .chain([TargetCellId::WindowsDesktopX64])
            .collect::<BTreeSet<_>>();
        let valid_boundary_targets = match self.boundary {
            HandoffBoundary::Hp1 => {
                self.target_cells == BTreeSet::from([TargetCellId::MacosDesktopArm64])
                    && self.windows_recheck_requirement == WindowsRecheckRequirement::NotApplicable
            }
            HandoffBoundary::Hp2 => {
                (self.target_cells == hp2_base
                    && self.windows_recheck_requirement
                        == WindowsRecheckRequirement::ReuseExactTuplePass)
                    || (self.target_cells == hp2_with_windows
                        && matches!(
                            self.windows_recheck_requirement,
                            WindowsRecheckRequirement::FullGate
                                | WindowsRecheckRequirement::NativeScopedAttestation
                        ))
            }
            HandoffBoundary::C8RecheckN | HandoffBoundary::C10RecheckN => {
                self.target_cells == required_target_cell_ids()
                    && matches!(
                        self.windows_recheck_requirement,
                        WindowsRecheckRequirement::FullGate
                            | WindowsRecheckRequirement::NativeScopedAttestation
                    )
            }
        };
        if !valid_boundary_targets {
            return Err(ValidationContractError::HandoffBoundaryTargets);
        }
        if !is_portable_repo_relative_path(&self.return_ledger_relative_path)
            || self
                .native_commands
                .iter()
                .any(|command| !is_portable_repo_relative_path(&command.working_directory))
        {
            return Err(ValidationContractError::NonPortableHandoffPath);
        }
        Ok(())
    }
}

impl PlatformValidationManifestPayload {
    pub fn validate_contract(&self) -> Result<(), ValidationContractError> {
        self.platform_matrix.validate_exact_contract()?;

        let required_cells = required_target_cell_ids();
        let mut check_ids = BTreeSet::new();
        let mut covered_cells = BTreeSet::new();
        for check in &self.required_checks {
            if !check_ids.insert(check.check_id.clone()) {
                return Err(ValidationContractError::DuplicateGateCheckId);
            }
            if !check.target_cells.is_subset(&required_cells) {
                return Err(ValidationContractError::UnknownCheckTargetCell);
            }
            covered_cells.extend(check.target_cells.iter().copied());
        }
        if covered_cells != required_cells {
            return Err(ValidationContractError::MissingTargetCellCheck);
        }

        let mut point_ids = BTreeSet::new();
        for point in &self.platform_verification_points {
            if !point_ids.insert(point.verification_point_id.clone()) {
                return Err(ValidationContractError::DuplicateVerificationPointId);
            }
            if !required_cells.contains(&point.target_cell) {
                return Err(ValidationContractError::UnknownVerificationPointCell);
            }
            if !check_ids.contains(&point.exact_check_id) {
                return Err(ValidationContractError::UnknownVerificationPointCheck);
            }
        }
        Ok(())
    }
}

impl D026OrderingOutcomeMatrix {
    pub fn validate_exact_contract(&self) -> bool {
        let cases = self
            .outcomes
            .iter()
            .map(|outcome| outcome.case_kind)
            .collect::<BTreeSet<_>>();
        self.outcomes.len() == 3
            && cases
                == BTreeSet::from([
                    D026OrderingCaseKind::RequestAdmissionCommittedBeforeFence,
                    D026OrderingCaseKind::FenceCommittedBeforeOldCredentialAdmission,
                    D026OrderingCaseKind::ReplacementCredentialAfterFence,
                ])
            && self.outcomes.iter().all(|outcome| {
            !outcome.existing_session_mutated
                && !outcome.existing_binding_mutated
                && !outcome.cascade_cancelled
                && match outcome.case_kind {
                    D026OrderingCaseKind::RequestAdmissionCommittedBeforeFence => {
                        outcome.outcome
                            == D026AdmissionOutcome::ContinuePreviouslyAdmittedOperationToFiniteBoundary
                            && outcome.expected_error_code.is_none()
                    }
                    D026OrderingCaseKind::FenceCommittedBeforeOldCredentialAdmission => {
                        outcome.outcome
                            == D026AdmissionOutcome::RejectRemoteAuthRequiredBeforeBindingOrSessionLookup
                            && outcome.expected_error_code
                                == Some(CanonicalErrorCode(
                                    "REMOTE_AUTH_REQUIRED".to_owned(),
                                ))
                    }
                    D026OrderingCaseKind::ReplacementCredentialAfterFence => {
                        outcome.outcome
                            == D026AdmissionOutcome::ContinueExistingSessionForSameOwnerWithExplicitSessionId
                            && outcome.expected_error_code.is_none()
                            && outcome.explicit_agent_session_id_required
                    }
                }
        })
    }
}

impl D027TerminalSequenceMatrix {
    pub fn validate_exact_contract(&self) -> bool {
        let cases = self
            .sequences
            .iter()
            .map(|sequence| sequence.case_kind)
            .collect::<BTreeSet<_>>();
        self.sequences.len() == 2
            && cases
                == BTreeSet::from([
                D027DrainCaseKind::NoDurableAcceptedOperation,
                D027DrainCaseKind::DurableAcceptedOperation,
                ])
            && self.sequences.iter().all(|sequence| {
                let expected_steps = match sequence.case_kind {
                    D027DrainCaseKind::NoDurableAcceptedOperation => vec![
                        D027TerminalStep::StopNomiAdmission,
                        D027TerminalStep::Cancel,
                        D027TerminalStep::DisposeRuntime,
                        D027TerminalStep::KillDescendants,
                        D027TerminalStep::ProveOutstandingExactZero,
                        D027TerminalStep::D024DeleteAgentSession,
                    ],
                    D027DrainCaseKind::DurableAcceptedOperation => vec![
                        D027TerminalStep::StopNomiAdmission,
                        D027TerminalStep::WaitExistingDeadlineMinimum,
                        D027TerminalStep::Cancel,
                        D027TerminalStep::DisposeRuntime,
                        D027TerminalStep::KillDescendants,
                        D027TerminalStep::DurableUncertainHandoff,
                        D027TerminalStep::ProveOutstandingExactZero,
                        D027TerminalStep::D024DeleteAgentSession,
                    ],
                };
                let expected_deadline = match sequence.case_kind {
                    D027DrainCaseKind::NoDurableAcceptedOperation => D027DeadlineRule::Immediate,
                    D027DrainCaseKind::DurableAcceptedOperation => {
                        D027DeadlineRule::MinimumOfOperationAndAllAncestorExistingFiniteDeadlines
                    }
                };
                sequence.steps == expected_steps
                    && sequence.deadline_rule == expected_deadline
                    && !sequence.handoff_waits_for_reconcile
                    && !sequence.same_session_runtime_switch_allowed
                    && !sequence.configurable_drain_timeout_allowed
                    && sequence.outstanding_after.is_exact_zero()
            })
    }
}

impl D028PlatformMatrix {
    pub fn validate_exact_contract(&self) -> Result<(), ValidationContractError> {
        if self.target_cells.keys().copied().collect::<BTreeSet<_>>() != required_target_cell_ids()
        {
            return Err(ValidationContractError::RequiredTargetCellSet);
        }
        if self.unsupported_local_targets != unsupported_local_target_exact_set() {
            return Err(ValidationContractError::UnsupportedTargetSet);
        }
        if self.remote_only_surfaces != remote_only_surface_exact_set() {
            return Err(ValidationContractError::RemoteOnlySurfaceSet);
        }
        if self.relay_order != canonical_platform_relay_order() {
            return Err(ValidationContractError::PlatformRelayOrder);
        }
        if self.recheck_policy != canonical_whole_cohort_recheck_policy() {
            return Err(ValidationContractError::RecheckPolicy);
        }
        for (cell_id, cell) in &self.target_cells {
            validate_target_cell(*cell_id, cell)?;
        }
        Ok(())
    }
}

pub fn required_target_cell_ids() -> BTreeSet<TargetCellId> {
    BTreeSet::from([
        TargetCellId::WindowsDesktopX64,
        TargetCellId::MacosDesktopArm64,
        TargetCellId::MacosDesktopX64,
        TargetCellId::LinuxDesktopX64,
        TargetCellId::LinuxHeadlessX64,
    ])
}

pub fn unsupported_local_target_exact_set() -> BTreeSet<UnsupportedLocalTarget> {
    BTreeSet::from([
        UnsupportedLocalTarget::WindowsArm64,
        UnsupportedLocalTarget::LinuxArm64,
    ])
}

pub fn remote_only_surface_exact_set() -> BTreeSet<RemoteOnlySurface> {
    BTreeSet::from([
        RemoteOnlySurface::Mobile,
        RemoteOnlySurface::WebBrowserClient,
        RemoteOnlySurface::RobotFirmware,
        RemoteOnlySurface::ImClient,
    ])
}

pub fn canonical_platform_relay_order() -> Vec<PlatformRelayStage> {
    vec![
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::WindowsC1C7Continuous,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C8WinPre,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::Hp1,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C8Ma,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::Hp2,
        },
        PlatformRelayStage::ParallelBoundaries {
            boundaries: BTreeSet::from([
                ValidationBoundary::C8Mx,
                ValidationBoundary::C8Ld,
                ValidationBoundary::C8Lh,
            ]),
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::MergeC8WholeBatchFixes,
        },
        PlatformRelayStage::RepeatableWholeCohortRecheck {
            family: RecheckFamily::C8,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C8Merge,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::D027FinalDrainExactZero,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C9NomiHardDelete,
        },
        PlatformRelayStage::ParallelBoundaries {
            boundaries: BTreeSet::from([
                ValidationBoundary::C10Win,
                ValidationBoundary::C10Ma,
                ValidationBoundary::C10Mx,
                ValidationBoundary::C10Ld,
                ValidationBoundary::C10Lh,
            ]),
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::MergeC10WholeBatchFixes,
        },
        PlatformRelayStage::RepeatableWholeCohortRecheck {
            family: RecheckFamily::C10,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C10Merge,
        },
        PlatformRelayStage::Boundary {
            boundary: ValidationBoundary::C11SameDigestStable,
        },
    ]
}

pub fn canonical_whole_cohort_recheck_policy() -> WholeCohortRecheckPolicy {
    WholeCohortRecheckPolicy {
        only_after_complete_round_returns: true,
        merge_fixes_before_freezing_new_tuple: true,
        affected_cells_run_full_gate: true,
        unaffected_cells_run_native_scoped_attestation: true,
        central_owner_cannot_attest_for_native_host: true,
        single_fix_platform_handoff_forbidden: true,
    }
}

fn validate_target_cell(
    cell_id: TargetCellId,
    cell: &TargetCell,
) -> Result<(), ValidationContractError> {
    let expected = match cell_id {
        TargetCellId::WindowsDesktopX64 => (
            HostOperatingSystem::Windows,
            HostArchitecture::X86_64,
            "x86_64-pc-windows-msvc",
            "x86_64-pc-windows-msvc",
            HostSurface::Desktop,
            "nsis",
            CapabilityAvailability::ReleaseManifestDefined,
            CapabilityAvailability::ReleaseManifestDefined,
        ),
        TargetCellId::MacosDesktopArm64 => (
            HostOperatingSystem::Macos,
            HostArchitecture::Aarch64,
            "aarch64-apple-darwin",
            "aarch64-apple-darwin",
            HostSurface::Desktop,
            "universal-app",
            CapabilityAvailability::ReleaseManifestDefined,
            CapabilityAvailability::ReleaseManifestDefined,
        ),
        TargetCellId::MacosDesktopX64 => (
            HostOperatingSystem::Macos,
            HostArchitecture::X86_64,
            "x86_64-apple-darwin",
            "x86_64-apple-darwin",
            HostSurface::Desktop,
            "universal-app",
            CapabilityAvailability::ReleaseManifestDefined,
            CapabilityAvailability::ReleaseManifestDefined,
        ),
        TargetCellId::LinuxDesktopX64 => (
            HostOperatingSystem::Linux,
            HostArchitecture::X86_64,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            HostSurface::Desktop,
            "appimage-deb-rpm",
            CapabilityAvailability::ReleaseManifestDefined,
            CapabilityAvailability::IndependentPartialOrExactUnavailable,
        ),
        TargetCellId::LinuxHeadlessX64 => (
            HostOperatingSystem::Linux,
            HostArchitecture::X86_64,
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            HostSurface::Headless,
            "headless-service",
            CapabilityAvailability::ExactUnavailable {
                error_code: CanonicalErrorCode("CAPABILITY_UNAVAILABLE_ON_PLATFORM".to_owned()),
            },
            CapabilityAvailability::ExactUnavailable {
                error_code: CanonicalErrorCode("CAPABILITY_UNAVAILABLE_ON_PLATFORM".to_owned()),
            },
        ),
    };

    if cell.host_os != expected.0
        || cell.host_arch != expected.1
        || cell.host_target.as_ref() != expected.2
        || cell.runtime_target.as_ref() != expected.3
        || cell.host_surface != expected.4
        || cell.package_format != expected.5
    {
        return Err(ValidationContractError::TargetCellDefinition(cell_id));
    }

    let expected_capabilities = BTreeMap::from([
        (
            CapabilityContractSubject::CodingCodexNative,
            CapabilityAvailability::RequiredExactSet,
        ),
        (CapabilityContractSubject::Browser, expected.6),
        (CapabilityContractSubject::Computer, expected.7),
    ]);
    if cell.capability_availability != expected_capabilities {
        return Err(ValidationContractError::CapabilityAvailability(cell_id));
    }
    Ok(())
}

fn is_portable_repo_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value.contains(":\\")
        && !value.contains(":/")
        && value
            .split(['/', '\\'])
            .all(|component| !matches!(component, "" | "." | ".."))
}

#[derive(Debug, Error)]
pub enum ValidationContractError {
    #[error("required target cell exact-set does not match D-028")]
    RequiredTargetCellSet,
    #[error("unsupported local target exact-set does not match D-028")]
    UnsupportedTargetSet,
    #[error("Remote-only surface exact-set does not match D-028")]
    RemoteOnlySurfaceSet,
    #[error("platform relay order does not match HP/recheck contract")]
    PlatformRelayOrder,
    #[error("whole-cohort recheck policy does not match contract")]
    RecheckPolicy,
    #[error("target cell definition is invalid for {0:?}")]
    TargetCellDefinition(TargetCellId),
    #[error("capability availability is invalid for {0:?}")]
    CapabilityAvailability(TargetCellId),
    #[error("duplicate required Gate check id")]
    DuplicateGateCheckId,
    #[error("required Gate check references an unknown target cell")]
    UnknownCheckTargetCell,
    #[error("one or more required target cells have no Gate check")]
    MissingTargetCellCheck,
    #[error("duplicate PlatformVerificationPoint id")]
    DuplicateVerificationPointId,
    #[error("PlatformVerificationPoint references an unknown target cell")]
    UnknownVerificationPointCell,
    #[error("PlatformVerificationPoint references an unknown Gate check")]
    UnknownVerificationPointCheck,
    #[error("candidate source change requires an impact assessment")]
    SourceImpactRequired,
    #[error("candidate source change requires at least one affected cell")]
    AffectedCellsRequired,
    #[error("impact assessment contains an unknown affected cell")]
    UnknownAffectedCell,
    #[error("handoff checkpoint worktree is not clean")]
    DirtyHandoffCheckpoint,
    #[error("handoff local, remote, and cohort source SHA differ")]
    HandoffShaMismatch,
    #[error("handoff references an unknown target cell")]
    UnknownHandoffTargetCell,
    #[error("handoff target cells do not match its HP/recheck boundary")]
    HandoffBoundaryTargets,
    #[error("handoff contains a machine absolute or non-portable path")]
    NonPortableHandoffPath,
}
