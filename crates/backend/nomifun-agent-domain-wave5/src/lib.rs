//! Bundled Agent Capability Platform v2 registrations for the automation,
//! supervision, and Remote domain wave.
//!
//! This crate intentionally depends only on the contract and thin-kernel
//! crates.  The domain registrations are source-neutral metadata plus pure,
//! deterministic handlers; production service wiring is supplied by the host
//! through the typed ports declared by each registration.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, CapabilityActionDescriptor, CapabilityContributions,
    CapabilityId, CapabilityKind, CapabilityManifest, CancellationDescriptor,
    CanonicalErrorCode, CanonicalSchemaRef, D026AdmissionOutcome, D026OrderingCaseKind,
    D026OrderingOutcome, D026OrderingOutcomeMatrix, D027DeadlineRule, D027DrainCaseKind,
    D027OutstandingSet, D027TerminalSequence, D027TerminalSequenceMatrix, D027TerminalStep,
    DeclaredServiceViewDescriptor, DomainOutboxPortDescriptor, EffectClass,
    HostPortBindingDescriptor, HostPortId, HostPortRef, InProcessEntrypointMetadata,
    LocalizedMetadata, ManagedTaskRegistrationDescriptor, PackageContributions, PackageId,
    PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality, PluginBootState,
    PluginContextDescriptor, PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor,
    PluginMountId, PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    RemoteAuthMutation, RemoteOperation, ResourceKind, ScopeKey, StrictJsonValue,
    RuntimeTarget, TypedCommandPortDescriptor, VersionString, REMOTE_AUTH_REQUIRED,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};

pub const VERSION: &str = "1.0.0";
pub const CONTRACT_VERSION: &str = VERSION;
pub const PACKAGE_VERSION: &str = VERSION;
const SOURCE_KIND: PluginSourceKind = PluginSourceKind::Bundled;

pub const AGENT_EXECUTION_PACKAGE: &str = "nomifun.agent-execution";
pub const AUTOWORK_SCHEDULER_PACKAGE: &str = "nomifun.autowork-scheduler";
pub const IDMM_PACKAGE: &str = "nomifun.idmm";
pub const REMOTE_INGRESS_PACKAGE: &str = "nomifun.remote-ingress";
pub const REQUIREMENTS_PACKAGE: &str = "nomifun.requirements";

pub const AGENT_EXECUTION_PACKAGE_ID: &str = AGENT_EXECUTION_PACKAGE;
pub const AUTOWORK_SCHEDULER_PACKAGE_ID: &str = AUTOWORK_SCHEDULER_PACKAGE;
pub const IDMM_PACKAGE_ID: &str = IDMM_PACKAGE;
pub const REMOTE_INGRESS_PACKAGE_ID: &str = REMOTE_INGRESS_PACKAGE;
pub const REQUIREMENTS_PACKAGE_ID: &str = REQUIREMENTS_PACKAGE;

pub const AGENT_DELEGATE: &str = "agent.delegate";
pub const AGENT_FORK: &str = "agent.fork";
pub const AGENT_EXECUTION_PLAN: &str = "agent.execution.plan";
pub const AGENT_EXECUTION_STEER: &str = "agent.execution.steer";
pub const AGENT_EXECUTION_OBSERVE: &str = "agent.execution.observe";

pub const AUTOWORK_RUNNER: &str = "autowork.runner";
pub const SCHEDULE_STORE: &str = "schedule.store";
pub const SCHEDULE_TIMER: &str = "schedule.timer";
pub const SCHEDULE_AGENT_TRIGGER: &str = "schedule.agent_trigger";

pub const IDMM_OBSERVE: &str = "idmm.observe";
pub const IDMM_INTERVENE: &str = "idmm.intervene";
pub const IDMM_FALLBACK_POLICY: &str = "idmm.fallback_policy";

pub const REMOTE_MCP: &str = "remote.mcp";
pub const REMOTE_REST: &str = "remote.rest";
pub const INGRESS_WEB: &str = "ingress.web";
pub const INGRESS_MOBILE: &str = "ingress.mobile";
pub const INGRESS_CHANNEL: &str = "ingress.channel";

pub const REQUIREMENTS_READ: &str = "requirements.read";
pub const REQUIREMENTS_WRITE: &str = "requirements.write";
pub const REQUIREMENTS_STATUS: &str = "requirements.status";
pub const REQUIREMENTS_CLAIM: &str = "requirements.claim";

pub const AGENT_DELEGATE_ACTION: &str = "agent.delegate.invoke";
pub const AGENT_FORK_ACTION: &str = "agent.fork.invoke";
pub const AGENT_EXECUTION_PLAN_ACTION: &str = "agent.execution.plan.invoke";
pub const AGENT_EXECUTION_STEER_ACTION: &str = "agent.execution.steer.invoke";
pub const AGENT_EXECUTION_OBSERVE_ACTION: &str = "agent.execution.observe.invoke";
pub const SCHEDULE_STORE_ACTION: &str = "schedule.store.invoke";
pub const REQUIREMENTS_READ_ACTION: &str = "requirements.read.invoke";
pub const REQUIREMENTS_WRITE_ACTION: &str = "requirements.write.invoke";
pub const REQUIREMENTS_STATUS_ACTION: &str = "requirements.status.invoke";
pub const REQUIREMENTS_CLAIM_ACTION: &str = "requirements.claim.invoke";

pub const REMOTE_OPEN_ACTION: &str = "remote.open";
pub const REMOTE_TURN_ACTION: &str = "remote.turn";
pub const REMOTE_OBSERVE_ACTION: &str = "remote.observe";
pub const REMOTE_CANCEL_ACTION: &str = "remote.cancel";

pub const TARGET_CAPABILITY_FAMILIES: [&str; 10] = [
    "agent-execution",
    "autowork.runner",
    "idmm.intervene",
    "idmm.observe",
    "remote.mcp",
    "remote.rest",
    "requirements",
    "schedule.agent-trigger",
    "schedule.store",
    "schedule.timer",
];

pub const PACKAGE_IDS: [&str; 5] = [
    AGENT_EXECUTION_PACKAGE,
    AUTOWORK_SCHEDULER_PACKAGE,
    IDMM_PACKAGE,
    REMOTE_INGRESS_PACKAGE,
    REQUIREMENTS_PACKAGE,
];
pub const TARGET_PACKAGE_IDS: [&str; 5] = PACKAGE_IDS;

const REMOTE_TRANSPORT_PORT: &str = "remote.transport";
const REMOTE_ADMISSION_PORT: &str = "remote.admission";
const REMOTE_DRAIN_PORT: &str = "remote.drain";
const REMOTE_OPEN_PORT: &str = "remote.open";
const REMOTE_TURN_PORT: &str = "remote.turn";
const REMOTE_OBSERVE_PORT: &str = "remote.observe";
const REMOTE_CANCEL_PORT: &str = "remote.cancel";

/// The exact capability IDs owned by the five target packages in the frozen
/// first-party inventory.
pub const TARGET_CAPABILITY_IDS: [&str; 21] = [
    AGENT_DELEGATE,
    AGENT_FORK,
    AGENT_EXECUTION_PLAN,
    AGENT_EXECUTION_STEER,
    AGENT_EXECUTION_OBSERVE,
    AUTOWORK_RUNNER,
    SCHEDULE_STORE,
    SCHEDULE_TIMER,
    SCHEDULE_AGENT_TRIGGER,
    IDMM_OBSERVE,
    IDMM_INTERVENE,
    IDMM_FALLBACK_POLICY,
    REMOTE_MCP,
    REMOTE_REST,
    INGRESS_WEB,
    INGRESS_MOBILE,
    INGRESS_CHANNEL,
    REQUIREMENTS_READ,
    REQUIREMENTS_WRITE,
    REQUIREMENTS_STATUS,
    REQUIREMENTS_CLAIM,
];
pub const ALL_CAPABILITY_IDS: [&str; 21] = TARGET_CAPABILITY_IDS;

pub const AGENT_EXECUTION_CAPABILITY_IDS: [&str; 5] = [
    AGENT_DELEGATE,
    AGENT_FORK,
    AGENT_EXECUTION_PLAN,
    AGENT_EXECUTION_STEER,
    AGENT_EXECUTION_OBSERVE,
];
pub const AUTOWORK_CAPABILITY_IDS: [&str; 4] = [
    AUTOWORK_RUNNER,
    SCHEDULE_STORE,
    SCHEDULE_TIMER,
    SCHEDULE_AGENT_TRIGGER,
];
pub const IDMM_CAPABILITY_IDS: [&str; 3] =
    [IDMM_OBSERVE, IDMM_INTERVENE, IDMM_FALLBACK_POLICY];
pub const REMOTE_INGRESS_CAPABILITY_IDS: [&str; 5] = [
    REMOTE_MCP,
    REMOTE_REST,
    INGRESS_WEB,
    INGRESS_MOBILE,
    INGRESS_CHANNEL,
];
pub const REQUIREMENTS_CAPABILITY_IDS: [&str; 4] = [
    REQUIREMENTS_READ,
    REQUIREMENTS_WRITE,
    REQUIREMENTS_STATUS,
    REQUIREMENTS_CLAIM,
];

/// The four Remote operations that are admitted by the canonical transport.
pub const REMOTE_OPERATION_IDS: [&str; 4] = [
    REMOTE_OPEN_ACTION,
    REMOTE_TURN_ACTION,
    REMOTE_OBSERVE_ACTION,
    REMOTE_CANCEL_ACTION,
];

/// A typed view of the Remote transport contribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTransportDescriptor {
    pub capability_ids: BTreeSet<CapabilityId>,
    pub operations: BTreeSet<RemoteOperation>,
    pub binding_fields: BTreeSet<String>,
    pub forbidden_binding_fields: BTreeSet<String>,
    pub transport_port: HostPortRef,
    pub admission_port: HostPortRef,
    pub drain_port: HostPortRef,
    pub typed_command_ports: Vec<TypedCommandPortDescriptor>,
    pub transport_only: bool,
    pub local_runtime_required: bool,
    pub explicit_session_id_for_follow_up: bool,
}

/// A typed view of the D-026 request-admission fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAdmissionDescriptor {
    pub ordering: D026OrderingOutcomeMatrix,
    pub request_operations: BTreeSet<RemoteOperation>,
    pub auth_mutations: BTreeSet<RemoteAuthMutation>,
    pub forbidden_auth_state: BTreeSet<String>,
    pub rejected_after_fence_code: CanonicalErrorCode,
    pub binding_mutation_count: u32,
    pub session_mutation_count: u32,
    pub effect_replay_count: u32,
    pub replacement_requires_same_owner: bool,
    pub replacement_requires_explicit_session_id: bool,
    pub implicit_lookup_allowed: bool,
}

/// A typed view of the D-027 finite drain contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDrainDescriptor {
    pub sequences: D027TerminalSequenceMatrix,
    pub exact_zero_before_delete: bool,
    pub configurable_timeout_allowed: bool,
    pub same_session_runtime_switch_allowed: bool,
    pub handoff_waits_for_reconcile: bool,
}

/// Release-time availability for the Remote ingress capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAvailabilityDescriptor {
    pub capability_ids: BTreeSet<CapabilityId>,
    pub supported_surfaces: BTreeSet<String>,
    pub remote_only_surfaces: BTreeSet<String>,
    pub supported_platforms: Vec<PlatformConstraint>,
    pub transport_only: bool,
    pub local_runtime_required: bool,
}

/// Compatibility aliases for callers that name the descriptors by their
/// contract layer rather than their domain role.
pub type TypedRemoteDescriptor = RemoteTransportDescriptor;
pub type TypedAdmissionDescriptor = RemoteAdmissionDescriptor;
pub type TypedDrainDescriptor = RemoteDrainDescriptor;
pub type RemoteBindingDescriptor = RemoteTransportDescriptor;
pub type AdmissionDescriptor = RemoteAdmissionDescriptor;
pub type DrainDescriptor = RemoteDrainDescriptor;

impl RemoteAvailabilityDescriptor {
    pub fn is_available_on(&self, surface: &str) -> bool {
        self.supported_surfaces.contains(surface)
    }

    pub fn is_remote_only(&self) -> bool {
        !self.local_runtime_required && self.transport_only
    }

    pub fn is_remote_client_surface(&self, surface: &str) -> bool {
        self.remote_only_surfaces.contains(surface)
    }
}

impl RemoteTransportDescriptor {
    pub fn supports_operation(&self, operation: RemoteOperation) -> bool {
        self.operations.contains(&operation)
    }

    pub fn port_for_operation(
        &self,
        operation: RemoteOperation,
    ) -> Option<&TypedCommandPortDescriptor> {
        let index = match operation {
            RemoteOperation::Open => 0,
            RemoteOperation::Turn => 1,
            RemoteOperation::Observe => 2,
            RemoteOperation::Cancel => 3,
        };
        self.typed_command_ports.get(index)
    }
}

impl RemoteDrainDescriptor {
    pub fn is_exact_contract(&self) -> bool {
        self.sequences.validate_exact_contract()
            && self.exact_zero_before_delete
            && !self.configurable_timeout_allowed
            && !self.same_session_runtime_switch_allowed
            && !self.handoff_waits_for_reconcile
    }
}

/// Return the five bundled registrations owned by Wave 5.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    Ok(vec![
        agent_execution_registration()?,
        autowork_registration()?,
        idmm_registration()?,
        remote_ingress_registration()?,
        requirements_registration()?,
    ])
}

pub fn agent_execution_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AGENT_EXECUTION_PACKAGE,
        "nomifun-agent-execution",
        agent_execution_capabilities(),
        agent_execution_ports(),
    )
}

pub fn autowork_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AUTOWORK_SCHEDULER_PACKAGE,
        "nomifun-autowork-scheduler",
        autowork_capabilities(),
        autowork_ports(),
    )
}

pub fn idmm_registration() -> Result<PluginRegistration, String> {
    registration_for(
        IDMM_PACKAGE,
        "nomifun-idmm",
        idmm_capabilities(),
        idmm_ports(),
    )
}

pub fn remote_ingress_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REMOTE_INGRESS_PACKAGE,
        "nomifun-remote-ingress",
        remote_capabilities(),
        remote_ports(),
    )
}

pub fn requirements_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REQUIREMENTS_PACKAGE,
        "nomifun-requirements",
        requirements_capabilities(),
        requirements_ports(),
    )
}

/// Return the target IDs as contract newtypes.
pub fn target_capability_ids() -> BTreeSet<CapabilityId> {
    TARGET_CAPABILITY_IDS
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

pub fn capability_ids() -> BTreeSet<CapabilityId> {
    target_capability_ids()
}

/// Return the package IDs in deterministic registration order.
pub fn package_ids() -> BTreeSet<PackageId> {
    PACKAGE_IDS.into_iter().map(PackageId::from).collect()
}

pub fn capability_ids_by_package() -> BTreeMap<PackageId, BTreeSet<CapabilityId>> {
    BTreeMap::from([
        (
            PackageId::from(AGENT_EXECUTION_PACKAGE),
            AGENT_EXECUTION_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect(),
        ),
        (
            PackageId::from(AUTOWORK_SCHEDULER_PACKAGE),
            AUTOWORK_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect(),
        ),
        (
            PackageId::from(IDMM_PACKAGE),
            IDMM_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect(),
        ),
        (
            PackageId::from(REMOTE_INGRESS_PACKAGE),
            REMOTE_INGRESS_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect(),
        ),
        (
            PackageId::from(REQUIREMENTS_PACKAGE),
            REQUIREMENTS_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect(),
        ),
    ])
}

pub fn required_resource_kinds(id: &str) -> Option<BTreeSet<ResourceKind>> {
    target_capability_ids()
        .contains(&CapabilityId::from(id))
        .then(BTreeSet::new)
}

/// Map a deletion-contract family to its canonical catalog IDs.
pub fn canonical_capability_ids_for_family(family: &str) -> BTreeSet<CapabilityId> {
    let ids: &[&str] = match family {
        "agent-execution" => &[
            AGENT_DELEGATE,
            AGENT_FORK,
            AGENT_EXECUTION_PLAN,
            AGENT_EXECUTION_STEER,
            AGENT_EXECUTION_OBSERVE,
        ],
        "autowork.runner" => &[AUTOWORK_RUNNER],
        "idmm.observe" => &[IDMM_OBSERVE],
        "idmm.intervene" => &[IDMM_INTERVENE],
        "idmm.fallback_policy" => &[IDMM_FALLBACK_POLICY],
        "remote.mcp" => &[REMOTE_MCP],
        "remote.rest" => &[REMOTE_REST],
        "ingress.web" => &[INGRESS_WEB],
        "ingress.mobile" => &[INGRESS_MOBILE],
        "ingress.channel" => &[INGRESS_CHANNEL],
        "requirements" => &[
            REQUIREMENTS_READ,
            REQUIREMENTS_WRITE,
            REQUIREMENTS_STATUS,
            REQUIREMENTS_CLAIM,
        ],
        "schedule.agent-trigger" => &[SCHEDULE_AGENT_TRIGGER],
        "schedule.store" => &[SCHEDULE_STORE],
        "schedule.timer" => &[SCHEDULE_TIMER],
        _ => &[],
    };
    ids.iter().map(|id| CapabilityId::from(*id)).collect()
}

/// Return the canonical action identity for an action-bearing Wave 5
/// capability.  Middleware and transport contributions intentionally have no
/// model action.
pub fn action_id(id: &str) -> Option<ActionId> {
    let action = match id {
        AGENT_DELEGATE => AGENT_DELEGATE_ACTION,
        AGENT_FORK => AGENT_FORK_ACTION,
        AGENT_EXECUTION_PLAN => AGENT_EXECUTION_PLAN_ACTION,
        AGENT_EXECUTION_STEER => AGENT_EXECUTION_STEER_ACTION,
        AGENT_EXECUTION_OBSERVE => AGENT_EXECUTION_OBSERVE_ACTION,
        SCHEDULE_STORE => SCHEDULE_STORE_ACTION,
        REQUIREMENTS_READ => REQUIREMENTS_READ_ACTION,
        REQUIREMENTS_WRITE => REQUIREMENTS_WRITE_ACTION,
        REQUIREMENTS_STATUS => REQUIREMENTS_STATUS_ACTION,
        REQUIREMENTS_CLAIM => REQUIREMENTS_CLAIM_ACTION,
        _ => return None,
    };
    Some(ActionId::from(action))
}

/// Check the surface portion of the Remote availability contract.  Remote
/// transport does not add a host-target branch; every target uses the same
/// transport declaration.
pub fn check_remote_availability(
    _host_target: &RuntimeTarget,
    surface: &str,
) -> Result<(), KernelError> {
    if remote_availability_descriptor().is_available_on(surface) {
        Ok(())
    } else {
        Err(KernelError::CapabilityUnavailableOnSurface {
            capability_id: CapabilityId::from(REMOTE_MCP),
            surface: surface.to_owned(),
        })
    }
}

pub fn remote_binding_field_names() -> BTreeSet<String> {
    remote_transport_descriptor().binding_fields
}

pub fn remote_binding_protocol_fixture(
) -> nomifun_agent_contracts::RemoteBindingProtocolFixture {
    nomifun_agent_contracts::remote_binding_protocol_fixture()
}

pub fn d026_request_admission_fixture(
) -> nomifun_agent_contracts::D026RequestAdmissionFixturePayload {
    nomifun_agent_contracts::d026_request_admission_fixture()
}

/// Return the D-026 ordering matrix, copied from the canonical outcome rules.
pub fn d026_ordering_descriptor() -> D026OrderingOutcomeMatrix {
    D026OrderingOutcomeMatrix {
        schema_version: VersionString::from(VERSION),
        outcomes: vec![
            D026OrderingOutcome {
                case_kind: D026OrderingCaseKind::RequestAdmissionCommittedBeforeFence,
                outcome: D026AdmissionOutcome::ContinuePreviouslyAdmittedOperationToFiniteBoundary,
                expected_error_code: None,
                existing_session_mutated: false,
                existing_binding_mutated: false,
                cascade_cancelled: false,
                explicit_agent_session_id_required: false,
            },
            D026OrderingOutcome {
                case_kind: D026OrderingCaseKind::FenceCommittedBeforeOldCredentialAdmission,
                outcome:
                    D026AdmissionOutcome::RejectRemoteAuthRequiredBeforeBindingOrSessionLookup,
                expected_error_code: Some(CanonicalErrorCode::from(REMOTE_AUTH_REQUIRED)),
                existing_session_mutated: false,
                existing_binding_mutated: false,
                cascade_cancelled: false,
                explicit_agent_session_id_required: false,
            },
            D026OrderingOutcome {
                case_kind: D026OrderingCaseKind::ReplacementCredentialAfterFence,
                outcome: D026AdmissionOutcome::ContinueExistingSessionForSameOwnerWithExplicitSessionId,
                expected_error_code: None,
                existing_session_mutated: false,
                existing_binding_mutated: false,
                cascade_cancelled: false,
                explicit_agent_session_id_required: true,
            },
        ],
    }
}

/// Return the exact D-027 terminal sequence matrix.
pub fn d027_drain_descriptor() -> D027TerminalSequenceMatrix {
    D027TerminalSequenceMatrix {
        schema_version: VersionString::from(VERSION),
        sequences: vec![
            D027TerminalSequence {
                case_kind: D027DrainCaseKind::NoDurableAcceptedOperation,
                deadline_rule: D027DeadlineRule::Immediate,
                steps: vec![
                    D027TerminalStep::StopNomiAdmission,
                    D027TerminalStep::Cancel,
                    D027TerminalStep::DisposeRuntime,
                    D027TerminalStep::KillDescendants,
                    D027TerminalStep::ProveOutstandingExactZero,
                    D027TerminalStep::D024DeleteAgentSession,
                ],
                handoff_waits_for_reconcile: false,
                same_session_runtime_switch_allowed: false,
                configurable_drain_timeout_allowed: false,
                outstanding_after: D027OutstandingSet::default(),
            },
            D027TerminalSequence {
                case_kind: D027DrainCaseKind::DurableAcceptedOperation,
                deadline_rule: D027DeadlineRule::MinimumOfOperationAndAllAncestorExistingFiniteDeadlines,
                steps: vec![
                    D027TerminalStep::StopNomiAdmission,
                    D027TerminalStep::WaitExistingDeadlineMinimum,
                    D027TerminalStep::Cancel,
                    D027TerminalStep::DisposeRuntime,
                    D027TerminalStep::KillDescendants,
                    D027TerminalStep::DurableUncertainHandoff,
                    D027TerminalStep::ProveOutstandingExactZero,
                    D027TerminalStep::D024DeleteAgentSession,
                ],
                handoff_waits_for_reconcile: false,
                same_session_runtime_switch_allowed: false,
                configurable_drain_timeout_allowed: false,
                outstanding_after: D027OutstandingSet::default(),
            },
        ],
    }
}

/// Return the typed Remote transport/admission/drain view used by the package.
pub fn remote_transport_descriptor() -> RemoteTransportDescriptor {
    let operations = BTreeSet::from([
        RemoteOperation::Open,
        RemoteOperation::Turn,
        RemoteOperation::Observe,
        RemoteOperation::Cancel,
    ]);
    RemoteTransportDescriptor {
        capability_ids: BTreeSet::from([
            CapabilityId::from(REMOTE_MCP),
            CapabilityId::from(REMOTE_REST),
            CapabilityId::from(INGRESS_WEB),
            CapabilityId::from(INGRESS_MOBILE),
            CapabilityId::from(INGRESS_CHANNEL),
        ]),
        operations,
        binding_fields: BTreeSet::from([
            "remote_binding_id".to_owned(),
            "owner_user_id".to_owned(),
            "name".to_owned(),
            "agent_binding".to_owned(),
        ]),
        forbidden_binding_fields: remote_binding_protocol_fixture()
            .forbidden_remote_binding_fields,
        transport_port: host_port(REMOTE_TRANSPORT_PORT),
        admission_port: host_port(REMOTE_ADMISSION_PORT),
        drain_port: host_port(REMOTE_DRAIN_PORT),
        typed_command_ports: vec![
            command_port(REMOTE_OPEN_PORT, "remote.open"),
            command_port(REMOTE_TURN_PORT, "remote.turn"),
            command_port(REMOTE_OBSERVE_PORT, "remote.observe"),
            command_port(REMOTE_CANCEL_PORT, "remote.cancel"),
        ],
        transport_only: true,
        local_runtime_required: false,
        explicit_session_id_for_follow_up: true,
    }
}

pub fn typed_remote_descriptor() -> RemoteTransportDescriptor {
    remote_transport_descriptor()
}

pub fn remote_binding_descriptor() -> RemoteBindingDescriptor {
    remote_transport_descriptor()
}

pub fn remote_admission_descriptor() -> RemoteAdmissionDescriptor {
    RemoteAdmissionDescriptor {
        ordering: d026_ordering_descriptor(),
        request_operations: BTreeSet::from([
            RemoteOperation::Open,
            RemoteOperation::Turn,
            RemoteOperation::Observe,
            RemoteOperation::Cancel,
        ]),
        auth_mutations: BTreeSet::from([RemoteAuthMutation::Rotate, RemoteAuthMutation::Revoke]),
        forbidden_auth_state: d026_request_admission_fixture().forbidden_auth_state,
        rejected_after_fence_code: CanonicalErrorCode::from(REMOTE_AUTH_REQUIRED),
        binding_mutation_count: 0,
        session_mutation_count: 0,
        effect_replay_count: 0,
        replacement_requires_same_owner: true,
        replacement_requires_explicit_session_id: true,
        implicit_lookup_allowed: false,
    }
}

pub fn typed_admission_descriptor() -> RemoteAdmissionDescriptor {
    remote_admission_descriptor()
}

pub fn admission_descriptor() -> AdmissionDescriptor {
    remote_admission_descriptor()
}

pub fn remote_drain_descriptor() -> RemoteDrainDescriptor {
    RemoteDrainDescriptor {
        sequences: d027_drain_descriptor(),
        exact_zero_before_delete: true,
        configurable_timeout_allowed: false,
        same_session_runtime_switch_allowed: false,
        handoff_waits_for_reconcile: false,
    }
}

pub fn typed_drain_descriptor() -> RemoteDrainDescriptor {
    remote_drain_descriptor()
}

pub fn drain_descriptor() -> DrainDescriptor {
    remote_drain_descriptor()
}

pub fn remote_availability_descriptor() -> RemoteAvailabilityDescriptor {
    RemoteAvailabilityDescriptor {
        capability_ids: BTreeSet::from([
            CapabilityId::from(REMOTE_MCP),
            CapabilityId::from(REMOTE_REST),
            CapabilityId::from(INGRESS_WEB),
            CapabilityId::from(INGRESS_MOBILE),
            CapabilityId::from(INGRESS_CHANNEL),
        ]),
        supported_surfaces: BTreeSet::from([
            "channel".to_owned(),
            "desktop".to_owned(),
            "headless".to_owned(),
            "im".to_owned(),
            "im-client".to_owned(),
            "mobile".to_owned(),
            "remote".to_owned(),
            "robot".to_owned(),
            "robot-firmware".to_owned(),
            "web".to_owned(),
            "web-browser-client".to_owned(),
        ]),
        remote_only_surfaces: BTreeSet::from([
            "im-client".to_owned(),
            "mobile".to_owned(),
            "robot-firmware".to_owned(),
            "web-browser-client".to_owned(),
        ]),
        supported_platforms: vec![PlatformConstraint::Any],
        transport_only: true,
        local_runtime_required: false,
    }
}

pub fn remote_availability() -> RemoteAvailabilityDescriptor {
    remote_availability_descriptor()
}

pub fn remote_only_surfaces() -> BTreeSet<String> {
    remote_availability_descriptor().remote_only_surfaces
}

pub fn remote_forbidden_binding_fields() -> BTreeSet<String> {
    remote_transport_descriptor().forbidden_binding_fields
}

struct CapabilitySpec {
    id: &'static str,
    kind: CapabilityKind,
    effect: EffectClass,
    surfaces: &'static [&'static str],
    host_ports: &'static [&'static str],
    actions: &'static [&'static str],
}

struct PortSpec {
    host_ports: &'static [&'static str],
    command_ports: &'static [&'static str],
    outbox_ports: &'static [&'static str],
}

const GENERAL_SURFACES: &[&str] = &["desktop", "headless", "remote"];
const REMOTE_SURFACES: &[&str] = &[
    "channel",
    "desktop",
    "headless",
    "im",
    "im-client",
    "mobile",
    "remote",
    "robot",
    "robot-firmware",
    "web",
    "web-browser-client",
];

const AGENT_PORTS: PortSpec = PortSpec {
    host_ports: &["agent-execution.dispatch"],
    command_ports: &["agent-execution.session-command"],
    outbox_ports: &["agent-execution.outbox"],
};

const AUTOWORK_PORTS: PortSpec = PortSpec {
    host_ports: &["autowork.scheduler"],
    command_ports: &["autowork.agent-trigger"],
    outbox_ports: &["autowork.outbox"],
};

const IDMM_PORTS: PortSpec = PortSpec {
    host_ports: &["idmm.supervision"],
    command_ports: &["idmm.intervention"],
    outbox_ports: &["idmm.outbox"],
};

const REMOTE_PORTS: PortSpec = PortSpec {
    host_ports: &[
        REMOTE_TRANSPORT_PORT,
        REMOTE_ADMISSION_PORT,
        REMOTE_DRAIN_PORT,
    ],
    command_ports: &[
        REMOTE_OPEN_PORT,
        REMOTE_TURN_PORT,
        REMOTE_OBSERVE_PORT,
        REMOTE_CANCEL_PORT,
    ],
    outbox_ports: &[],
};

const REQUIREMENTS_PORTS: PortSpec = PortSpec {
    host_ports: &["requirements.board"],
    command_ports: &["requirements.command"],
    outbox_ports: &["requirements.outbox"],
};

const fn tool_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Tool,
        effect: EffectClass::WriteReversible,
        surfaces: GENERAL_SURFACES,
        host_ports: &[],
        actions: &[],
    }
}

const fn scheduler_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Scheduler,
        effect: EffectClass::WriteDurable,
        surfaces: GENERAL_SURFACES,
        host_ports: &[],
        actions: &[],
    }
}

const fn middleware_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::TurnMiddleware,
        effect: EffectClass::ReadLocal,
        surfaces: GENERAL_SURFACES,
        host_ports: &[],
        actions: &[],
    }
}

const fn remote_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Transport,
        effect: EffectClass::ExternalTransmit,
        surfaces: REMOTE_SURFACES,
        host_ports: &[
            REMOTE_TRANSPORT_PORT,
            REMOTE_ADMISSION_PORT,
            REMOTE_DRAIN_PORT,
        ],
        actions: &[],
    }
}

fn agent_execution_capabilities() -> Vec<CapabilitySpec> {
    vec![
        CapabilitySpec {
            actions: &[AGENT_DELEGATE_ACTION],
            effect: EffectClass::WriteDurable,
            ..tool_spec(AGENT_DELEGATE)
        },
        CapabilitySpec {
            actions: &[AGENT_FORK_ACTION],
            effect: EffectClass::WriteDurable,
            ..tool_spec(AGENT_FORK)
        },
        CapabilitySpec {
            actions: &[AGENT_EXECUTION_PLAN_ACTION],
            effect: EffectClass::WriteDurable,
            ..tool_spec(AGENT_EXECUTION_PLAN)
        },
        CapabilitySpec {
            actions: &[AGENT_EXECUTION_STEER_ACTION],
            ..tool_spec(AGENT_EXECUTION_STEER)
        },
        CapabilitySpec {
            actions: &[AGENT_EXECUTION_OBSERVE_ACTION],
            effect: EffectClass::ReadLocal,
            ..tool_spec(AGENT_EXECUTION_OBSERVE)
        },
    ]
}

fn autowork_capabilities() -> Vec<CapabilitySpec> {
    vec![
        scheduler_spec(AUTOWORK_RUNNER),
        CapabilitySpec {
            actions: &[SCHEDULE_STORE_ACTION],
            effect: EffectClass::WriteDurable,
            ..tool_spec(SCHEDULE_STORE)
        },
        scheduler_spec(SCHEDULE_TIMER),
        scheduler_spec(SCHEDULE_AGENT_TRIGGER),
    ]
}

fn idmm_capabilities() -> Vec<CapabilitySpec> {
    vec![
        middleware_spec(IDMM_OBSERVE),
        middleware_spec(IDMM_INTERVENE),
        middleware_spec(IDMM_FALLBACK_POLICY),
    ]
}

fn remote_capabilities() -> Vec<CapabilitySpec> {
    vec![
        remote_spec(REMOTE_MCP),
        remote_spec(REMOTE_REST),
        remote_spec(INGRESS_WEB),
        remote_spec(INGRESS_MOBILE),
        remote_spec(INGRESS_CHANNEL),
    ]
}

fn requirements_capabilities() -> Vec<CapabilitySpec> {
    vec![
        CapabilitySpec {
            actions: &[REQUIREMENTS_READ_ACTION],
            effect: EffectClass::ReadLocal,
            ..tool_spec(REQUIREMENTS_READ)
        },
        CapabilitySpec {
            actions: &[REQUIREMENTS_WRITE_ACTION],
            ..tool_spec(REQUIREMENTS_WRITE)
        },
        CapabilitySpec {
            actions: &[REQUIREMENTS_STATUS_ACTION],
            effect: EffectClass::ReadLocal,
            ..tool_spec(REQUIREMENTS_STATUS)
        },
        CapabilitySpec {
            actions: &[REQUIREMENTS_CLAIM_ACTION],
            effect: EffectClass::WriteDurable,
            ..tool_spec(REQUIREMENTS_CLAIM)
        },
    ]
}

fn agent_execution_ports() -> PortSpec {
    AGENT_PORTS
}

fn autowork_ports() -> PortSpec {
    AUTOWORK_PORTS
}

fn idmm_ports() -> PortSpec {
    IDMM_PORTS
}

fn remote_ports() -> PortSpec {
    REMOTE_PORTS
}

fn requirements_ports() -> PortSpec {
    REQUIREMENTS_PORTS
}

fn registration_for(
    package_id: &'static str,
    mount_id: &'static str,
    capabilities: Vec<CapabilitySpec>,
    ports: PortSpec,
) -> Result<PluginRegistration, String> {
    let package = package_ref(package_id);
    let port_ids = all_port_ids(&ports);
    let capability_manifests = capabilities
        .iter()
        .map(|spec| capability_manifest(spec, &package, &port_ids))
        .collect::<Vec<_>>();
    let config_schema = schema_value();
    let manifest = PackageManifest {
        schema_version: VersionString::from(VERSION),
        host_contract_version: VersionString::from(VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(package_id, "Bundled Wave 5 automation domain package."),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{package_id}.entrypoint"),
            contract_version: VersionString::from(VERSION),
        },
        contributions: PackageContributions {
            capabilities: capability_manifests,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
        },
    };
    let source = PluginSourceMetadata {
        source_kind: SOURCE_KIND,
        source_identity: package_id.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: PluginMountId::from(mount_id),
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|error| error.to_string())?,
        mount_id: identity.mount_id.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::BindHostPort,
                PluginRegistrarOperation::ContributeCapability,
            ]),
            declared_capability_ids: capabilities
                .iter()
                .map(|spec| CapabilityId::from(spec.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: port_ids,
        },
        context: PluginContextDescriptor {
            identity: identity.clone(),
            source,
            validated_config: nomifun_agent_contracts::ValidatedPluginConfig {
                schema_digest: nomifun_agent_contracts::digest_payload(&config_schema)
                    .map_err(|error| error.to_string())?,
                config_revision: 1,
                value: empty_object(),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: identity.mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: ports
                .host_ports
                .iter()
                .map(|id| host_port_binding(id))
                .collect(),
            typed_command_ports: ports
                .command_ports
                .iter()
                .map(|id| command_port(id, id))
                .collect(),
            domain_outbox_ports: ports
                .outbox_ports
                .iter()
                .map(|id| outbox_port(id))
                .collect(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{mount_id}")),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{mount_id}")),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    for spec in &capabilities {
        if spec.actions.is_empty() {
            continue;
        }
        registration
            .add_capability_handler(
                CapabilityId::from(spec.id),
                Arc::new(DeterministicActionHandler {
                    capability_id: CapabilityId::from(spec.id),
                    action_ids: spec
                        .actions
                        .iter()
                        .map(|action| ActionId::from(*action))
                        .collect(),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(registration)
}

fn capability_manifest(
    spec: &CapabilitySpec,
    package: &PackageRef,
    declared_port_ids: &BTreeSet<HostPortId>,
) -> CapabilityManifest {
    let capability_id = CapabilityId::from(spec.id);
    let actions = spec
        .actions
        .iter()
        .map(|action| CapabilityActionDescriptor {
            action_id: ActionId::from(*action),
            input_schema: schema_ref(spec.id, "input"),
            output_schema: schema_ref(spec.id, "output"),
            effect_class: spec.effect,
            presentation: if spec.kind == CapabilityKind::Transport
                || spec.kind == CapabilityKind::Scheduler
                || spec.kind == CapabilityKind::TurnMiddleware
            {
                nomifun_agent_contracts::ToolPresentationKind::Hidden
            } else {
                nomifun_agent_contracts::ToolPresentationKind::FunctionTool
            },
        })
        .collect();
    let context_schema_refs = if spec.kind == CapabilityKind::TurnMiddleware {
        vec![schema_ref(spec.id, "context")]
    } else {
        Vec::new()
    };
    let event_schema_refs = if spec.kind == CapabilityKind::Scheduler {
        vec![schema_ref(spec.id, "event")]
    } else {
        Vec::new()
    };
    CapabilityManifest {
        id: capability_id,
        version: VersionString::from(VERSION),
        kind: spec.kind,
        package: package.clone(),
        display: display(spec.id, "Deterministic Wave 5 capability contribution."),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: spec
            .surfaces
            .iter()
            .map(|surface| (*surface).to_owned())
            .collect(),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: schema_value(),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs,
            event_schema_refs,
            resource_kinds: BTreeSet::<ResourceKind>::new(),
            host_ports: spec
                .host_ports
                .iter()
                .filter(|id| declared_port_ids.contains(&HostPortId::from(**id)))
                .map(|id| host_port(*id).clone())
                .collect(),
        },
    }
}

struct DeterministicActionHandler {
    capability_id: CapabilityId,
    action_ids: BTreeSet<ActionId>,
}

impl CapabilityHandler for DeterministicActionHandler {
    fn invoke<'life0, 'async_trait>(
        &'life0 self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, KernelError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            if context.capability_id != self.capability_id
                || !self.action_ids.contains(&context.action_id)
            {
                return Err(KernelError::ActionNotDeclared {
                    capability_id: context.capability_id,
                    action_id: context.action_id,
                });
            }
            if !input.0.is_object() {
                return Err(KernelError::CapabilityExecution {
                    reason: format!(
                        "{} input must be a JSON object",
                        self.capability_id.as_ref()
                    ),
                });
            }
            Ok(deterministic_result(
                &context.capability_id,
                &context.action_id,
                &context.resource_bindings,
                context.registry_generation,
                input,
            ))
        })
    }
}

fn deterministic_result(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    resource_bindings: &[nomifun_agent_contracts::TypedResourceBinding],
    registry_generation: u64,
    input: StrictJsonValue,
) -> StrictJsonValue {
    let mut result = empty_object();
    let object = result
        .0
        .as_object_mut()
        .expect("empty_object always returns a JSON object");
    let mut resource_binding_ids = resource_bindings
        .iter()
        .map(|binding| binding.binding_id.as_ref().to_owned())
        .collect::<Vec<_>>();
    resource_binding_ids.sort();
    object.insert("accepted".to_owned(), true.into());
    object.insert(
        "command".to_owned(),
        "agent-session.domain-dispatch".to_owned().into(),
    );
    object.insert(
        "action_id".to_owned(),
        action_id.as_ref().to_owned().into(),
    );
    object.insert(
        "capability_id".to_owned(),
        capability_id.as_ref().to_owned().into(),
    );
    object.insert("registry_generation".to_owned(), registry_generation.into());
    object.insert("resource_binding_ids".to_owned(), resource_binding_ids.into());
    object.insert("deterministic".to_owned(), true.into());
    object.insert("input".to_owned(), input.0);
    StrictJsonValue(result.0)
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn package_ref(package_id: &str) -> PackageRef {
    PackageRef {
        id: PackageId::from(package_id),
        version: VersionString::from(VERSION),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(VERSION),
    }
}

fn host_port_binding(id: &str) -> HostPortBindingDescriptor {
    HostPortBindingDescriptor {
        port: host_port(id),
        request_schema: schema_ref(id, "request"),
        response_schema: schema_ref(id, "response"),
    }
}

fn command_port(id: &str, schema_name: &str) -> TypedCommandPortDescriptor {
    TypedCommandPortDescriptor {
        port: host_port(id),
        command_schema: schema_ref(schema_name, "command"),
        receipt_schema: schema_ref(schema_name, "receipt"),
    }
}

fn outbox_port(id: &str) -> DomainOutboxPortDescriptor {
    DomainOutboxPortDescriptor {
        port: host_port(id),
        event_schema: schema_ref(id, "event"),
        cursor_schema: schema_ref(id, "cursor"),
    }
}

fn schema_value() -> StrictJsonValue {
    object_schema(false)
}

fn object_schema(additional_properties: bool) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty_object always returns a JSON object");
    object.insert(
        "additionalProperties".to_owned(),
        additional_properties.into(),
    );
    object.insert("type".to_owned(), "object".to_owned().into());
    StrictJsonValue(value.0)
}

fn empty_object() -> StrictJsonValue {
    let mut value = nomifun_agent_contracts::remote_binding_protocol_fixture()
        .open
        .request
        .initial_input
        .expect("the canonical Remote fixture supplies an object value")
        .0;
    value
        .as_object_mut()
        .expect("the canonical Remote fixture input is an object")
        .clear();
    StrictJsonValue(value)
}

fn schema_ref(subject: &str, role: &str) -> CanonicalSchemaRef {
    let additional_properties = role != "config";
    let digest = nomifun_agent_contracts::digest_payload(&object_schema(additional_properties))
        .expect("the built-in object schema is canonicalizable");
    CanonicalSchemaRef::from(format!(
        "schema://{subject}/{role}@{VERSION}#{}",
        digest.as_ref()
    ))
}

fn all_port_ids(ports: &PortSpec) -> BTreeSet<HostPortId> {
    ports
        .host_ports
        .iter()
        .chain(ports.command_ports.iter())
        .chain(ports.outbox_ports.iter())
        .chain(["host.plugin.cancel", "host.plugin.tasks"].iter())
        .map(|id| HostPortId::from(*id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_cover_the_exact_wave_five_target_set() {
        let registrations = registrations().expect("Wave 5 registrations must build");
        assert_eq!(registrations.len(), 5);

        let expected_packages = [
            AGENT_EXECUTION_PACKAGE,
            AUTOWORK_SCHEDULER_PACKAGE,
            IDMM_PACKAGE,
            REMOTE_INGRESS_PACKAGE,
            REQUIREMENTS_PACKAGE,
        ];
        let package_ids = registrations
            .iter()
            .map(|registration| registration.metadata.manifest.payload.package_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            package_ids,
            expected_packages
                .into_iter()
                .map(PackageId::from)
                .collect::<BTreeSet<_>>()
        );

        let capability_ids = registrations
            .iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.clone())
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(capability_ids, target_capability_ids());
        assert_eq!(
            capability_ids.len(),
            TARGET_CAPABILITY_IDS.len(),
            "every target capability must have one canonical owner"
        );
        assert!(registrations.iter().all(|registration| {
            registration
                .handler_ids()
                .iter()
                .all(|capability| capability_ids.contains(capability))
        }));
    }

    #[test]
    fn each_target_package_contains_its_complete_inventory_slice() {
        let registrations = registrations().unwrap();
        let expected = [
            (
                AGENT_EXECUTION_PACKAGE,
                [
                    AGENT_DELEGATE,
                    AGENT_FORK,
                    AGENT_EXECUTION_PLAN,
                    AGENT_EXECUTION_STEER,
                    AGENT_EXECUTION_OBSERVE,
                ]
                .as_slice(),
            ),
            (
                AUTOWORK_SCHEDULER_PACKAGE,
                [
                    AUTOWORK_RUNNER,
                    SCHEDULE_STORE,
                    SCHEDULE_TIMER,
                    SCHEDULE_AGENT_TRIGGER,
                ]
                .as_slice(),
            ),
            (
                IDMM_PACKAGE,
                [IDMM_OBSERVE, IDMM_INTERVENE, IDMM_FALLBACK_POLICY].as_slice(),
            ),
            (
                REMOTE_INGRESS_PACKAGE,
                [
                    REMOTE_MCP,
                    REMOTE_REST,
                    INGRESS_WEB,
                    INGRESS_MOBILE,
                    INGRESS_CHANNEL,
                ]
                .as_slice(),
            ),
            (
                REQUIREMENTS_PACKAGE,
                [
                    REQUIREMENTS_READ,
                    REQUIREMENTS_WRITE,
                    REQUIREMENTS_STATUS,
                    REQUIREMENTS_CLAIM,
                ]
                .as_slice(),
            ),
        ];

        for (registration, (package_id, expected_capabilities)) in
            registrations.iter().zip(expected)
        {
            let manifest = &registration.metadata.manifest.payload;
            assert_eq!(manifest.package_id.as_ref(), package_id);
            let actual = manifest
                .contributions
                .capabilities
                .iter()
                .map(|capability| capability.id.as_ref())
                .collect::<BTreeSet<_>>();
            let expected = expected_capabilities.iter().copied().collect::<BTreeSet<_>>();
            assert_eq!(actual, expected);

            for capability in &manifest.contributions.capabilities {
                if capability.kind == CapabilityKind::Tool {
                    assert_eq!(capability.contributions.actions.len(), 1);
                    assert!(registration.handler_ids().contains(&capability.id));
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(!registration.handler_ids().contains(&capability.id));
                }
            }
        }
    }

    #[test]
    fn remote_capabilities_are_transport_only_and_available_on_remote_surfaces() {
        let registrations = registrations().unwrap();
        let remote = &registrations[3];
        let capabilities = &remote.metadata.manifest.payload.contributions.capabilities;
        assert_eq!(capabilities.len(), 5);
        assert!(capabilities.iter().all(|capability| {
            capability.kind == CapabilityKind::Transport
                && capability
                    .supported_surfaces
                    .contains("web-browser-client")
                && capability.supported_platforms == vec![PlatformConstraint::Any]
        }));

        let availability = remote_availability_descriptor();
        assert!(availability.is_remote_only());
        assert!(availability.is_available_on("mobile"));
        assert!(availability.is_available_on("web-browser-client"));
        assert!(availability.is_available_on("robot-firmware"));
        assert!(availability.is_available_on("im-client"));
        assert!(availability.transport_only);
        assert!(!availability.local_runtime_required);
        assert!(availability.is_remote_client_surface("mobile"));
        assert!(!availability.is_remote_client_surface("desktop"));
    }

    #[test]
    fn remote_transport_exposes_only_the_canonical_binding_and_operations() {
        let descriptor = remote_transport_descriptor();
        assert_eq!(
            descriptor.capability_ids,
            REMOTE_INGRESS_CAPABILITY_IDS
                .into_iter()
                .map(CapabilityId::from)
                .collect()
        );
        assert_eq!(
            descriptor.operations,
            BTreeSet::from([
                RemoteOperation::Open,
                RemoteOperation::Turn,
                RemoteOperation::Observe,
                RemoteOperation::Cancel,
            ])
        );
        assert_eq!(
            descriptor.binding_fields,
            BTreeSet::from([
                "agent_binding".to_owned(),
                "name".to_owned(),
                "owner_user_id".to_owned(),
                "remote_binding_id".to_owned(),
            ])
        );
        assert_eq!(descriptor.typed_command_ports.len(), 4);
        for operation in [
            RemoteOperation::Open,
            RemoteOperation::Turn,
            RemoteOperation::Observe,
            RemoteOperation::Cancel,
        ] {
            assert!(descriptor.supports_operation(operation));
            assert!(descriptor.port_for_operation(operation).is_some());
        }
        assert!(descriptor
            .forbidden_binding_fields
            .iter()
            .all(|field| !descriptor.binding_fields.contains(field)));
    }

    #[test]
    fn admission_and_drain_descriptors_match_the_frozen_contracts() {
        let admission = remote_admission_descriptor();
        assert!(admission.ordering.validate_exact_contract());
        assert_eq!(
            admission.rejected_after_fence_code.as_ref(),
            REMOTE_AUTH_REQUIRED
        );
        assert_eq!(admission.binding_mutation_count, 0);
        assert_eq!(admission.session_mutation_count, 0);
        assert_eq!(admission.effect_replay_count, 0);
        assert!(admission.replacement_requires_same_owner);
        assert!(admission.replacement_requires_explicit_session_id);
        assert!(!admission.implicit_lookup_allowed);
        assert_eq!(
            admission.request_operations,
            BTreeSet::from([
                RemoteOperation::Open,
                RemoteOperation::Turn,
                RemoteOperation::Observe,
                RemoteOperation::Cancel,
            ])
        );
        assert_eq!(
            admission.auth_mutations,
            BTreeSet::from([RemoteAuthMutation::Rotate, RemoteAuthMutation::Revoke])
        );
        assert!(admission
            .forbidden_auth_state
            .iter()
            .all(|field| !field.is_empty()));

        let drain = remote_drain_descriptor();
        assert!(drain.is_exact_contract());
        assert_eq!(drain.sequences.sequences.len(), 2);
        assert!(drain.sequences.sequences.iter().all(|sequence| {
            sequence.outstanding_after.is_exact_zero()
        }));
    }

    #[test]
    fn registrations_materialize_and_export_only_action_handlers() {
        let registry = nomifun_agent_kernel::KernelRegistry::new(
            nomifun_agent_kernel::MaterializationPolicy::stable(VERSION),
            Arc::new(nomifun_agent_kernel::InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence should initialize");
        let materialized = registry
            .replace_all(registrations().expect("registrations should build"))
            .expect("Wave 5 metadata should materialize");

        assert_eq!(materialized.packages.len(), PACKAGE_IDS.len());
        assert_eq!(materialized.capabilities.len(), TARGET_CAPABILITY_IDS.len());
        for registration in registrations().expect("registrations should rebuild") {
            let declared_actions = registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .iter()
                .filter(|capability| !capability.contributions.actions.is_empty())
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.handler_ids(), declared_actions);
        }
    }

    #[test]
    fn handlers_return_stable_results_without_a_second_authority() {
        let registrations = registrations().unwrap();
        let agent_execution = &registrations[0];
        let capability = agent_execution
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .iter()
            .find(|capability| capability.id == CapabilityId::from(AGENT_DELEGATE))
            .unwrap();
        assert_eq!(
            capability
                .contributions
                .actions
                .iter()
                .map(|action| action.action_id.as_ref())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([AGENT_DELEGATE_ACTION])
        );
        assert!(agent_execution
            .handler_ids()
            .contains(&CapabilityId::from(AGENT_DELEGATE)));
        assert!(registrations[3].handler_ids().is_empty());
        assert!(remote_drain_descriptor().is_exact_contract());
    }
}
