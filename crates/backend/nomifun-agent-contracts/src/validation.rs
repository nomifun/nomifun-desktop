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

validation_string_newtype!(GateCheckId);
validation_string_newtype!(PlatformVerificationPointId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ConfirmedDecisionContractDigestRef(pub DigestHex);

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
    LinuxDesktopX64,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D028PlatformMatrix {
    pub target_cells: BTreeMap<TargetCellId, TargetCell>,
    pub unsupported_local_targets: BTreeSet<UnsupportedLocalTarget>,
    pub remote_only_surfaces: BTreeSet<RemoteOnlySurface>,
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

/// Pre-run immutable input. The source commit, its own digest, status, evidence,
/// logs, and summary are structurally absent and cannot participate in its
/// digest. The Gate attaches the clean source commit to post-run evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformValidationManifestPayload {
    pub manifest_version: VersionString,
    pub confirmed_decision_contract_digest: ConfirmedDecisionContractDigestRef,
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
        TargetCellId::LinuxDesktopX64,
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

#[derive(Debug, Error)]
pub enum ValidationContractError {
    #[error("required target cell exact-set does not match D-028")]
    RequiredTargetCellSet,
    #[error("unsupported local target exact-set does not match D-028")]
    UnsupportedTargetSet,
    #[error("Remote-only surface exact-set does not match D-028")]
    RemoteOnlySurfaceSet,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn pre_run_platform_manifest_rejects_source_commit() {
        let mut value: Value = serde_json::from_str(include_str!(
            "../contracts/validation/platform-validation-manifest.payload.json"
        ))
        .expect("parse pre-run PlatformValidationManifest fixture");

        assert!(
            value.get("candidate_source_sha").is_none(),
            "pre-run PlatformValidationManifest must not contain a source commit"
        );
        serde_json::from_value::<PlatformValidationManifestPayload>(value.clone())
            .expect("deserialize pre-run PlatformValidationManifest");

        value
            .as_object_mut()
            .expect("PlatformValidationManifest must be an object")
            .insert(
                "candidate_source_sha".to_owned(),
                json!("a".repeat(40)),
            );
        assert!(
            serde_json::from_value::<PlatformValidationManifestPayload>(value).is_err(),
            "legacy candidate_source_sha must be rejected"
        );
    }

    #[test]
    fn platform_matrix_has_no_cross_machine_handoff_contract() {
        let matrix: D028PlatformMatrix = serde_json::from_str(include_str!(
            "../contracts/validation/d028-platform-matrix.json"
        ))
        .expect("deserialize D-028 platform matrix");

        matrix
            .validate_exact_contract()
            .expect("D-028 matrix must match the canonical platform contract");
        assert_eq!(
            matrix.target_cells.keys().copied().collect::<BTreeSet<_>>(),
            required_target_cell_ids()
        );

        let serialized = serde_json::to_string(&matrix).expect("serialize D-028 platform matrix");
        for removed_term in ["hp-1", "hp-2", "handoff", "attestation"] {
            assert!(
                !serialized.contains(removed_term),
                "D-028 must not retain cross-machine term {removed_term:?}"
            );
        }
    }

}
