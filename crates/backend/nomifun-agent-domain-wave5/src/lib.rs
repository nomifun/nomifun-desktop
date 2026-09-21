//! Bundled Agent Capability Platform v2 registrations for the automation,
//! supervision, and Remote domain wave.
//!
//! This crate intentionally depends only on the contract and thin-kernel
//! crates.  The domain registrations are source-neutral metadata plus typed
//! handlers; production service wiring is supplied by the host
//! through the typed ports declared by each registration.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityAuthoringPolicy, CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind,
    CapabilityManifest,
    CancellationDescriptor, CanonicalErrorCode, CanonicalSchemaRef, CorrelationId,
    D026AdmissionOutcome, D026OrderingCaseKind, D026OrderingOutcome,
    D026OrderingOutcomeMatrix, D027DeadlineRule, D027DrainCaseKind, D027OutstandingSet,
    D027TerminalSequence, D027TerminalSequenceMatrix, D027TerminalStep,
    DeclaredServiceViewDescriptor, DomainOutboxPortDescriptor, EffectClass,
    HostPortBindingDescriptor, HostPortId, HostPortRef, IdempotencyKey,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor, PluginMountId,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    RemoteAuthMutation, RemoteOperation, ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    StrictJsonValue,
    TypedCommandPortDescriptor, TypedResourceBinding, TypedResourceBindings, RuntimeTarget,
    VersionString, REMOTE_AUTH_REQUIRED, capability_module_surface_declarations,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, DeclaredServiceView, KernelError,
    PluginRegistration, PluginStateHandle,
};

pub const VERSION: &str = "1.0.0";
pub const CONTRACT_VERSION: &str = VERSION;
pub const PACKAGE_VERSION: &str = VERSION;
const SOURCE_KIND: PluginSourceKind = PluginSourceKind::Bundled;

pub const AGENT_EXECUTION_PACKAGE: &str = "nomifun.agent-execution";
pub const AUTOMATION_SCHEDULE_PACKAGE: &str = "nomifun.automation-schedule";
pub const REMOTE_INGRESS_PACKAGE: &str = "nomifun.remote-ingress";
pub const REQUIREMENTS_PACKAGE: &str = "nomifun.requirements";

pub const AGENT_EXECUTION_PACKAGE_ID: &str = AGENT_EXECUTION_PACKAGE;
pub const AUTOMATION_SCHEDULE_PACKAGE_ID: &str = AUTOMATION_SCHEDULE_PACKAGE;
pub const REMOTE_INGRESS_PACKAGE_ID: &str = REMOTE_INGRESS_PACKAGE;
pub const REQUIREMENTS_PACKAGE_ID: &str = REQUIREMENTS_PACKAGE;
pub const REMOTE_INGRESS_MOUNT_ID: &str = "nomifun-remote-ingress";

/// Product Modules. AgentExecution plan/observe/steer are Runtime/role-derived
/// authorities and therefore are not authoring capabilities.
pub const AGENT_COLLABORATION_MODULE_ID: &str = "agent.collaboration";
pub const AUTOMATION_SCHEDULE_MODULE_ID: &str = "automation.schedule";
pub const REQUIREMENTS_MODULE_ID: &str = "requirements";

pub const AGENT_DELEGATE_ACTION_ID: &str = "agent/delegate";
pub const AGENT_FORK_ACTION_ID: &str = "agent/fork";
pub const AGENT_REQUEST_USER_DECISION_ACTION_ID: &str = "agent/request_user_decision";
pub const SCHEDULE_LIST_ACTION_ID: &str = "automation.schedule/list";
pub const SCHEDULE_CREATE_ACTION_ID: &str = "automation.schedule/create";
pub const SCHEDULE_UPDATE_ACTION_ID: &str = "automation.schedule/update";
pub const SCHEDULE_DELETE_ACTION_ID: &str = "automation.schedule/delete";
pub const REQUIREMENTS_READ_ACTION_ID: &str = "requirements/read";
pub const REQUIREMENTS_WRITE_ACTION_ID: &str = "requirements/write";
pub const REQUIREMENTS_STATUS_ACTION_ID: &str = "requirements/status";
pub const REQUIREMENTS_CLAIM_ACTION_ID: &str = "requirements/claim";

/// Platform-only Remote operation vocabulary. These are transport commands,
/// never Agent Module or Action grants.
pub const REMOTE_OPEN_ACTION: &str = "remote.open";
pub const REMOTE_TURN_ACTION: &str = "remote.turn";
pub const REMOTE_OBSERVE_ACTION: &str = "remote.observe";
pub const REMOTE_CANCEL_ACTION: &str = "remote.cancel";
pub const REMOTE_MCP: &str = "remote.ingress/mcp";
pub const REMOTE_REST: &str = "remote.ingress/rest";
pub const INGRESS_WEB: &str = "remote.ingress/web";
pub const INGRESS_MOBILE: &str = "remote.ingress/mobile";
pub const INGRESS_CHANNEL: &str = "remote.ingress/channel";

pub const SCHEDULER_RESOURCE_KIND: &str = "scheduler";
const SCHEDULER_RESOURCES: &[&str] = &[SCHEDULER_RESOURCE_KIND];

pub const PACKAGE_IDS: [&str; 4] = [
    AGENT_EXECUTION_PACKAGE,
    AUTOMATION_SCHEDULE_PACKAGE,
    REMOTE_INGRESS_PACKAGE,
    REQUIREMENTS_PACKAGE,
];
pub const TARGET_PACKAGE_IDS: [&str; 4] = PACKAGE_IDS;
pub const TARGET_CAPABILITY_IDS: [&str; 3] = [
    AGENT_COLLABORATION_MODULE_ID,
    AUTOMATION_SCHEDULE_MODULE_ID,
    REQUIREMENTS_MODULE_ID,
];
pub const ALL_CAPABILITY_IDS: [&str; 3] = TARGET_CAPABILITY_IDS;
pub const AGENT_EXECUTION_CAPABILITY_IDS: [&str; 1] = [AGENT_COLLABORATION_MODULE_ID];
pub const AUTOMATION_SCHEDULE_CAPABILITY_IDS: [&str; 1] = [AUTOMATION_SCHEDULE_MODULE_ID];
pub const REMOTE_INGRESS_CAPABILITY_IDS: [&str; 0] = [];
pub const REQUIREMENTS_CAPABILITY_IDS: [&str; 1] = [REQUIREMENTS_MODULE_ID];

pub const REMOTE_OPERATION_IDS: [&str; 4] = [
    REMOTE_OPEN_ACTION,
    REMOTE_TURN_ACTION,
    REMOTE_OBSERVE_ACTION,
    REMOTE_CANCEL_ACTION,
];

const REMOTE_TRANSPORT_PORT: &str = "remote.transport";
const REMOTE_ADMISSION_PORT: &str = "remote.admission";
const REMOTE_DRAIN_PORT: &str = "remote.drain";
const REMOTE_OPEN_PORT: &str = "remote.open";
const REMOTE_TURN_PORT: &str = "remote.turn";
const REMOTE_OBSERVE_PORT: &str = "remote.observe";
const REMOTE_CANCEL_PORT: &str = "remote.cancel";
/// The single host port for action-bearing Wave 5 capabilities.
///
/// Wave 5 owns the capability vocabulary and input boundary, while the
/// application owns AgentExecution, scheduling, and requirements facts.
/// Keeping this port in the domain crate avoids a dependency on the
/// application composition root and prevents a synthetic success result when
/// no owner has been wired.
pub const WAVE5_CAPABILITY_HOST_PORT_ID: &str = "host.wave5.capability.invoke";
pub const WAVE5_HOST_PORT_UNAVAILABLE: &str = "WAVE5_HOST_PORT_UNAVAILABLE";
pub const WAVE5_INVALID_REQUEST: &str = "WAVE5_INVALID_REQUEST";
pub const WAVE5_ACTION_OPERATION_MISMATCH: &str = "WAVE5_ACTION_OPERATION_MISMATCH";
pub const WAVE5_RESOURCE_BINDING_INVALID: &str = "WAVE5_RESOURCE_BINDING_INVALID";
pub const WAVE5_EFFECT_OUTCOME_UNKNOWN: &str = "WAVE5_EFFECT_OUTCOME_UNKNOWN";
pub const AGENT_EXECUTION_ALREADY_ACTIVE: &str = "AGENT_EXECUTION_ALREADY_ACTIVE";
const WAVE5_INVALID_RESPONSE: &str = "WAVE5_INVALID_RESPONSE";

/// Kernel-authorized invocation context projected to the application owner.
///
/// `state` is the already namespace-scoped Kernel handle for this package
/// mount. It is intentionally the only state surface exposed here: no raw
/// persistence, registry, database, or service bag crosses the boundary.
#[derive(Clone)]
pub struct Wave5HostContext {
    pub principal: nomifun_agent_contracts::PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub turn_id: OperationId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub state_scope_key: ScopeKey,
    pub state: PluginStateHandle,
    pub services: DeclaredServiceView,
    pub resource_bindings: TypedResourceBindings,
}

/// The owning domain fixed by a typed action operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave5OwnerDomain {
    AgentExecution,
    Schedule,
    Requirements,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave5CapabilityOperation {
    AgentDelegate { input: StrictJsonValue },
    AgentFork { input: StrictJsonValue },
    AgentRequestUserDecision { input: StrictJsonValue },
    ScheduleList { input: StrictJsonValue },
    ScheduleCreate { input: StrictJsonValue },
    ScheduleUpdate { input: StrictJsonValue },
    ScheduleDelete { input: StrictJsonValue },
    RequirementsRead { input: StrictJsonValue },
    RequirementsWrite { input: StrictJsonValue },
    RequirementsStatus { input: StrictJsonValue },
    RequirementsClaim { input: StrictJsonValue },
}

impl Wave5CapabilityOperation {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::AgentDelegate { .. }
            | Self::AgentFork { .. }
            | Self::AgentRequestUserDecision { .. } => {
                AGENT_COLLABORATION_MODULE_ID
            }
            Self::ScheduleList { .. }
            | Self::ScheduleCreate { .. }
            | Self::ScheduleUpdate { .. }
            | Self::ScheduleDelete { .. } => AUTOMATION_SCHEDULE_MODULE_ID,
            Self::RequirementsRead { .. }
            | Self::RequirementsWrite { .. }
            | Self::RequirementsStatus { .. }
            | Self::RequirementsClaim { .. } => REQUIREMENTS_MODULE_ID,
        })
    }

    pub fn action_id(&self) -> ActionId {
        ActionId::from(match self {
            Self::AgentDelegate { .. } => AGENT_DELEGATE_ACTION_ID,
            Self::AgentFork { .. } => AGENT_FORK_ACTION_ID,
            Self::AgentRequestUserDecision { .. } => AGENT_REQUEST_USER_DECISION_ACTION_ID,
            Self::ScheduleList { .. } => SCHEDULE_LIST_ACTION_ID,
            Self::ScheduleCreate { .. } => SCHEDULE_CREATE_ACTION_ID,
            Self::ScheduleUpdate { .. } => SCHEDULE_UPDATE_ACTION_ID,
            Self::ScheduleDelete { .. } => SCHEDULE_DELETE_ACTION_ID,
            Self::RequirementsRead { .. } => REQUIREMENTS_READ_ACTION_ID,
            Self::RequirementsWrite { .. } => REQUIREMENTS_WRITE_ACTION_ID,
            Self::RequirementsStatus { .. } => REQUIREMENTS_STATUS_ACTION_ID,
            Self::RequirementsClaim { .. } => REQUIREMENTS_CLAIM_ACTION_ID,
        })
    }

    pub fn owner_domain(&self) -> Wave5OwnerDomain {
        match self {
            Self::AgentDelegate { .. }
            | Self::AgentFork { .. }
            | Self::AgentRequestUserDecision { .. } => {
                Wave5OwnerDomain::AgentExecution
            }
            Self::ScheduleList { .. }
            | Self::ScheduleCreate { .. }
            | Self::ScheduleUpdate { .. }
            | Self::ScheduleDelete { .. } => Wave5OwnerDomain::Schedule,
            Self::RequirementsRead { .. }
            | Self::RequirementsWrite { .. }
            | Self::RequirementsStatus { .. }
            | Self::RequirementsClaim { .. } => Wave5OwnerDomain::Requirements,
        }
    }

    fn input(&self) -> &StrictJsonValue {
        match self {
            Self::AgentDelegate { input }
            | Self::AgentFork { input }
            | Self::AgentRequestUserDecision { input }
            | Self::ScheduleList { input }
            | Self::ScheduleCreate { input }
            | Self::ScheduleUpdate { input }
            | Self::ScheduleDelete { input }
            | Self::RequirementsRead { input }
            | Self::RequirementsWrite { input }
            | Self::RequirementsStatus { input }
            | Self::RequirementsClaim { input } => input,
        }
    }
}

#[derive(Clone)]
pub struct Wave5HostRequest {
    pub context: Wave5HostContext,
    pub operation: Wave5CapabilityOperation,
}

impl Wave5HostRequest {
    pub fn validate(&self) -> Result<(), Wave5HostPortError> {
        let capability_id = &self.context.capability_id;
        let Some(module) = module_spec(capability_id.as_ref()) else {
            return Err(Wave5HostPortError::invalid_request(format!(
                "unknown Wave 5 Module {}",
                capability_id.as_ref()
            )));
        };
        let Some(action) = action_spec(module.id, self.context.action_id.as_ref()) else {
            return Err(Wave5HostPortError::action_operation_mismatch(format!(
                "{} does not declare Action {}",
                capability_id.as_ref(),
                self.context.action_id.as_ref(),
            )));
        };
        if self.operation.capability_id() != *capability_id
            || self.operation.action_id() != self.context.action_id
        {
            return Err(Wave5HostPortError::action_operation_mismatch(format!(
                "context maps {} / {} but typed operation maps {} / {}",
                capability_id.as_ref(),
                self.context.action_id.as_ref(),
                self.operation.capability_id().as_ref(),
                self.operation.action_id().as_ref()
            )));
        }
        if !self.operation.input().0.is_object() {
            return Err(Wave5HostPortError::invalid_request(format!(
                "{} input must be a JSON object",
                capability_id.as_ref()
            )));
        }
        validate_host_context(&self.context)?;
        validate_resource_bindings_contract(
            capability_id,
            &self.context.principal.principal_id,
            action.requirements,
            &self.context.resource_bindings,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave5HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave5HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(WAVE5_HOST_PORT_UNAVAILABLE, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(WAVE5_INVALID_REQUEST, message)
    }

    pub fn action_operation_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE5_ACTION_OPERATION_MISMATCH, message)
    }

    pub fn resource_binding_invalid(message: impl Into<String>) -> Self {
        Self::new(WAVE5_RESOURCE_BINDING_INVALID, message)
    }

    pub fn canonical_code(&self) -> CanonicalErrorCode {
        CanonicalErrorCode::from(self.code.clone())
    }
}

impl fmt::Display for Wave5HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave5HostPortError {}

/// Application-owned implementation boundary for action-bearing Wave 5
/// capabilities.
pub trait Wave5HostPort: Send + Sync {
    fn invoke<'a>(
        &'a self,
        request: Wave5HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>>;
}

struct UnconfiguredWave5HostPort;

impl Wave5HostPort for UnconfiguredWave5HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave5HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            Err(Wave5HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

pub fn unconfigured_host_port() -> Arc<dyn Wave5HostPort> {
    Arc::new(UnconfiguredWave5HostPort)
}

/// Independently injectable product owners behind the single action port.
///
/// All fields are optional so central composition can wire owners in bounded
/// slices. A missing owner fails closed; no branch manufactures a receipt,
/// echoes the request as success, or falls back to another domain.
#[derive(Default)]
pub struct Wave5OwnerBindings {
    pub agent_execution: Option<Arc<dyn Wave5HostPort>>,
    pub schedule: Option<Arc<dyn Wave5HostPort>>,
    pub requirements: Option<Arc<dyn Wave5HostPort>>,
}

impl Wave5OwnerBindings {
    pub fn with_agent_execution(mut self, owner: Arc<dyn Wave5HostPort>) -> Self {
        self.agent_execution = Some(owner);
        self
    }

    pub fn with_schedule(mut self, owner: Arc<dyn Wave5HostPort>) -> Self {
        self.schedule = Some(owner);
        self
    }

    pub fn with_requirements(mut self, owner: Arc<dyn Wave5HostPort>) -> Self {
        self.requirements = Some(owner);
        self
    }
}

/// Compose real Wave 5 owners for injection through
/// [`registrations_with_host_port`].
pub fn composed_host_port(bindings: Wave5OwnerBindings) -> Arc<dyn Wave5HostPort> {
    Arc::new(ComposedWave5HostPort { bindings })
}

struct ComposedWave5HostPort {
    bindings: Wave5OwnerBindings,
}

impl Wave5HostPort for ComposedWave5HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave5HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>>
    {
        if let Err(error) = request.validate() {
            return Box::pin(async move { Err(error) });
        }
        let owner = match request.operation.owner_domain() {
            Wave5OwnerDomain::AgentExecution => self.bindings.agent_execution.clone(),
            Wave5OwnerDomain::Schedule => self.bindings.schedule.clone(),
            Wave5OwnerDomain::Requirements => self.bindings.requirements.clone(),
        };
        let capability_id = request.context.capability_id.clone();
        Box::pin(async move {
            let Some(owner) = owner else {
                return Err(Wave5HostPortError::unavailable(format!(
                    "no production owner is bound for {}",
                    capability_id.as_ref()
                )));
            };
            owner.invoke(request).await
        })
    }
}

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

impl RemoteAdmissionDescriptor {
    pub fn is_exact_contract(&self) -> bool {
        let fixture = d026_request_admission_fixture();
        self.ordering.schema_version == VersionString::from(VERSION)
            && self.ordering.validate_exact_contract()
            && self.request_operations == fixture.operation_exact_set
            && self.auth_mutations == fixture.auth_mutation_exact_set
            && self.forbidden_auth_state == fixture.forbidden_auth_state
            && self.rejected_after_fence_code == CanonicalErrorCode::from(REMOTE_AUTH_REQUIRED)
            && self.binding_mutation_count == 0
            && self.session_mutation_count == 0
            && self.effect_replay_count == 0
            && self.replacement_requires_same_owner
            && self.replacement_requires_explicit_session_id
            && !self.implicit_lookup_allowed
    }
}

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
        let port_id = match operation {
            RemoteOperation::Open => REMOTE_OPEN_PORT,
            RemoteOperation::Turn => REMOTE_TURN_PORT,
            RemoteOperation::Observe => REMOTE_OBSERVE_PORT,
            RemoteOperation::Cancel => REMOTE_CANCEL_PORT,
        };
        self.typed_command_ports
            .iter()
            .find(|port| port.port.id.as_ref() == port_id)
    }

    pub fn is_exact_contract(&self) -> bool {
        let expected_capabilities = REMOTE_INGRESS_CAPABILITY_IDS
            .into_iter()
            .map(CapabilityId::from)
            .collect::<BTreeSet<_>>();
        let expected_operations = BTreeSet::from([
            RemoteOperation::Open,
            RemoteOperation::Turn,
            RemoteOperation::Observe,
            RemoteOperation::Cancel,
        ]);
        let expected_binding_fields = BTreeSet::from([
            "agent_binding".to_owned(),
            "name".to_owned(),
            "owner_user_id".to_owned(),
            "remote_binding_id".to_owned(),
        ]);
        let command_port_ids = self
            .typed_command_ports
            .iter()
            .map(|port| port.port.id.as_ref())
            .collect::<BTreeSet<_>>();
        self.capability_ids == expected_capabilities
            && self.operations == expected_operations
            && self.binding_fields == expected_binding_fields
            && self.forbidden_binding_fields
                == remote_binding_protocol_fixture().forbidden_remote_binding_fields
            && self.transport_port == host_port(REMOTE_TRANSPORT_PORT)
            && self.admission_port == host_port(REMOTE_ADMISSION_PORT)
            && self.drain_port == host_port(REMOTE_DRAIN_PORT)
            && command_port_ids
                == BTreeSet::from([
                    REMOTE_OPEN_PORT,
                    REMOTE_TURN_PORT,
                    REMOTE_OBSERVE_PORT,
                    REMOTE_CANCEL_PORT,
                ])
            && self.typed_command_ports.len() == REMOTE_OPERATION_IDS.len()
            && self.typed_command_ports.iter().all(|port| {
                *port == command_port(port.port.id.as_ref(), port.port.id.as_ref())
            })
            && self.transport_only
            && !self.local_runtime_required
            && self.explicit_session_id_for_follow_up
    }
}

impl RemoteDrainDescriptor {
    pub fn is_exact_contract(&self) -> bool {
        self.sequences.schema_version == VersionString::from(VERSION)
            && self.sequences.validate_exact_contract()
            && self.exact_zero_before_delete
            && !self.configurable_timeout_allowed
            && !self.same_session_runtime_switch_allowed
            && !self.handoff_waits_for_reconcile
    }
}

/// Return the three Agent Modules plus the non-grant Remote transport package.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave5HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    Ok(vec![
        registration_for(
            AGENT_EXECUTION_PACKAGE,
            "nomifun-agent-execution",
            agent_execution_modules(),
            agent_execution_ports(),
            Some(Arc::clone(&action_host_port)),
        )?,
        registration_for(
            AUTOMATION_SCHEDULE_PACKAGE,
            "nomifun-automation-schedule",
            automation_schedule_modules(),
            automation_schedule_ports(),
            Some(Arc::clone(&action_host_port)),
        )?,
        registration_for(
            REMOTE_INGRESS_PACKAGE,
            REMOTE_INGRESS_MOUNT_ID,
            Vec::new(),
            remote_ports(),
            None,
        )?,
        registration_for(
            REQUIREMENTS_PACKAGE,
            "nomifun-requirements",
            requirements_modules(),
            requirements_ports(),
            Some(action_host_port),
        )?,
    ])
}

pub fn agent_execution_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AGENT_EXECUTION_PACKAGE,
        "nomifun-agent-execution",
        agent_execution_modules(),
        agent_execution_ports(),
        Some(unconfigured_host_port()),
    )
}

pub fn automation_schedule_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AUTOMATION_SCHEDULE_PACKAGE,
        "nomifun-automation-schedule",
        automation_schedule_modules(),
        automation_schedule_ports(),
        Some(unconfigured_host_port()),
    )
}

pub fn remote_ingress_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REMOTE_INGRESS_PACKAGE,
        REMOTE_INGRESS_MOUNT_ID,
        Vec::new(),
        remote_ports(),
        None,
    )
}

pub fn requirements_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REQUIREMENTS_PACKAGE,
        "nomifun-requirements",
        requirements_modules(),
        requirements_ports(),
        Some(unconfigured_host_port()),
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

/// A typed resource slot used by the Wave 5 AgentExecution contribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedResourceDescriptor {
    pub slot_key: &'static str,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: &'static str,
}

/// Return the resource slots declared by this wave.
pub fn typed_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    vec![TypedResourceDescriptor {
            slot_key: SCHEDULER_RESOURCE_KIND,
            resource_kind: ResourceKind::from(SCHEDULER_RESOURCE_KIND),
            required: true,
            operations: BTreeSet::from([
                "delete".to_owned(),
                "read".to_owned(),
                "write".to_owned(),
            ]),
            binding_policy: "bind_one",
        }]
}

pub fn all_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

pub fn resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

/// Return the union of operations declared for each typed resource kind.
pub fn resource_binding_metadata() -> BTreeMap<ResourceKind, BTreeSet<String>> {
    typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect()
}

/// Build a deterministic fixture binding for the AgentExecution process lane.
///
/// This creates no process and does not resolve a product resource. It is only
/// a typed contract fixture for callers constructing an AgentBinding revision.
pub fn canonical_resource_bindings(owner_id: impl Into<String>) -> TypedResourceBindings {
    let owner_id = owner_id.into();
    vec![typed_resource_binding(
            "wave5-scheduler",
            SCHEDULER_RESOURCE_KIND,
            "installation-scheduler",
            &owner_id,
            ["delete", "read", "write"],
        )]
}

pub fn resource_bindings(owner_id: impl Into<String>) -> TypedResourceBindings {
    canonical_resource_bindings(owner_id)
}

pub fn typed_resource_binding<I, S>(
    binding_id: impl Into<ResourceBindingId>,
    resource_kind: impl Into<ResourceKind>,
    resource_id: impl Into<ResourceId>,
    owner_id: impl Into<String>,
    operations: I,
) -> TypedResourceBinding
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    TypedResourceBinding {
        binding_id: binding_id.into(),
        resource_kind: resource_kind.into(),
        resource_id: resource_id.into(),
        owner_id: owner_id.into(),
        operations: operations.into_iter().map(Into::into).collect(),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
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
            PackageId::from(AUTOMATION_SCHEDULE_PACKAGE),
            AUTOMATION_SCHEDULE_CAPABILITY_IDS
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
    module_spec(id).map(|spec| {
        spec.actions
            .iter()
            .flat_map(|action| action.resource_kinds.iter())
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
}

pub fn required_action_resource_operations(
    module_id: &str,
    action_id: &str,
) -> Option<Vec<(ResourceKind, String)>> {
    action_spec(module_id, action_id).map(|action| {
        action
            .requirements
            .iter()
            .map(|requirement| {
                (
                    ResourceKind::from(requirement.resource_kind),
                    requirement.operation.to_owned(),
                )
            })
            .collect()
    })
}

/// Resolve an exact Action ID to its product Module.
pub fn module_id_for_action(action_id: &str) -> Option<CapabilityId> {
    let module = match action_id {
        AGENT_DELEGATE_ACTION_ID
        | AGENT_FORK_ACTION_ID
        | AGENT_REQUEST_USER_DECISION_ACTION_ID => AGENT_COLLABORATION_MODULE_ID,
        SCHEDULE_LIST_ACTION_ID
        | SCHEDULE_CREATE_ACTION_ID
        | SCHEDULE_UPDATE_ACTION_ID
        | SCHEDULE_DELETE_ACTION_ID => AUTOMATION_SCHEDULE_MODULE_ID,
        REQUIREMENTS_READ_ACTION_ID
        | REQUIREMENTS_WRITE_ACTION_ID
        | REQUIREMENTS_STATUS_ACTION_ID
        | REQUIREMENTS_CLAIM_ACTION_ID => REQUIREMENTS_MODULE_ID,
        _ => return None,
    };
    Some(CapabilityId::from(module))
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
        capability_ids: BTreeSet::new(),
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
        capability_ids: BTreeSet::new(),
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

#[derive(Clone, Copy)]
struct ActionSpec {
    id: &'static str,
    effect: EffectClass,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
}

#[derive(Clone, Copy)]
struct ModuleSpec {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    actions: &'static [ActionSpec],
}

#[derive(Clone, Copy)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

struct PortSpec {
    host_ports: &'static [&'static str],
    command_ports: &'static [&'static str],
    outbox_ports: &'static [&'static str],
}

const GENERAL_SURFACES: &[&str] = &["desktop", "headless", "remote"];

const AGENT_PORTS: PortSpec = PortSpec {
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID, "agent-execution.dispatch"],
    command_ports: &["agent-execution.session-command"],
    outbox_ports: &["agent-execution.outbox"],
};

const AUTOMATION_SCHEDULE_PORTS: PortSpec = PortSpec {
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID],
    command_ports: &[],
    outbox_ports: &[],
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
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID, "requirements.board"],
    command_ports: &["requirements.command"],
    outbox_ports: &["requirements.outbox"],
};

const SCHEDULER_READ_REQUIREMENT: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: SCHEDULER_RESOURCE_KIND,
    operation: "read",
}];
const SCHEDULER_WRITE_REQUIREMENT: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: SCHEDULER_RESOURCE_KIND,
    operation: "write",
}];
const SCHEDULER_DELETE_REQUIREMENT: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: SCHEDULER_RESOURCE_KIND,
    operation: "delete",
}];

const AGENT_COLLABORATION_ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: AGENT_DELEGATE_ACTION_ID,
        effect: EffectClass::ExecuteLocal,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: AGENT_FORK_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: AGENT_REQUEST_USER_DECISION_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
    },
];

const AUTOMATION_SCHEDULE_ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: SCHEDULE_LIST_ACTION_ID,
        effect: EffectClass::ReadSensitive,
        resource_kinds: SCHEDULER_RESOURCES,
        requirements: SCHEDULER_READ_REQUIREMENT,
    },
    ActionSpec {
        id: SCHEDULE_CREATE_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: SCHEDULER_RESOURCES,
        requirements: SCHEDULER_WRITE_REQUIREMENT,
    },
    ActionSpec {
        id: SCHEDULE_UPDATE_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: SCHEDULER_RESOURCES,
        requirements: SCHEDULER_WRITE_REQUIREMENT,
    },
    ActionSpec {
        id: SCHEDULE_DELETE_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: SCHEDULER_RESOURCES,
        requirements: SCHEDULER_DELETE_REQUIREMENT,
    },
];

const REQUIREMENTS_ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: REQUIREMENTS_READ_ACTION_ID,
        effect: EffectClass::ReadSensitive,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: REQUIREMENTS_WRITE_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: REQUIREMENTS_STATUS_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: REQUIREMENTS_CLAIM_ACTION_ID,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
    },
];

const MODULES: &[ModuleSpec] = &[
    ModuleSpec {
        id: AGENT_COLLABORATION_MODULE_ID,
        display_name: "Agent Collaboration",
        description: "Create one persistent AgentExecution. For parallel fan-out, call agent/delegate exactly once with {\"strategy\":\"parallel\",\"tasks\":[{\"name\":\"upstream-a\",\"prompt\":\"...\"},{\"name\":\"upstream-b\",\"prompt\":\"...\"}],\"synthesize\":true}. The field is tasks (plural) and must be a JSON array; synthesize is a JSON boolean, never a string. true adds one downstream Agent that receives every upstream result. For one automatically planned goal use {\"strategy\":\"planned\",\"goal\":\"...\"}. Use agent/fork only for one isolated delegated Agent; never issue sibling collaboration calls concurrently.",
        actions: AGENT_COLLABORATION_ACTIONS,
    },
    ModuleSpec {
        id: AUTOMATION_SCHEDULE_MODULE_ID,
        display_name: "Automation Schedule",
        description: "List and explicitly mutate durable automation schedules.",
        actions: AUTOMATION_SCHEDULE_ACTIONS,
    },
    ModuleSpec {
        id: REQUIREMENTS_MODULE_ID,
        display_name: "Requirements",
        description: "Read and explicitly mutate long-lived product requirements.",
        actions: REQUIREMENTS_ACTIONS,
    },
];

fn agent_execution_modules() -> Vec<ModuleSpec> {
    vec![MODULES[0]]
}

fn automation_schedule_modules() -> Vec<ModuleSpec> {
    vec![MODULES[1]]
}

fn requirements_modules() -> Vec<ModuleSpec> {
    vec![MODULES[2]]
}

fn agent_execution_ports() -> PortSpec {
    AGENT_PORTS
}

fn automation_schedule_ports() -> PortSpec {
    AUTOMATION_SCHEDULE_PORTS
}

fn remote_ports() -> PortSpec {
    REMOTE_PORTS
}

fn requirements_ports() -> PortSpec {
    REQUIREMENTS_PORTS
}

fn module_spec(id: &str) -> Option<&'static ModuleSpec> {
    MODULES.iter().find(|module| module.id == id)
}

fn action_spec(module_id: &str, action_id: &str) -> Option<&'static ActionSpec> {
    module_spec(module_id)?
        .actions
        .iter()
        .find(|action| action.id == action_id)
}

fn registration_for(
    package_id: &'static str,
    mount_id: &'static str,
    modules: Vec<ModuleSpec>,
    ports: PortSpec,
    action_host_port: Option<Arc<dyn Wave5HostPort>>,
) -> Result<PluginRegistration, String> {
    let package = package_ref(package_id);
    let port_ids = all_port_ids(&ports);
    let capability_manifests = modules
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
        }
        .into(),
        contributions: PackageContributions {
            capabilities: capability_manifests,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
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
            allowed_operations: if modules.is_empty() {
                BTreeSet::from([PluginRegistrarOperation::BindHostPort])
            } else {
                BTreeSet::from([
                    PluginRegistrarOperation::BindHostPort,
                    PluginRegistrarOperation::ContributeCapability,
                ])
            },
            declared_capability_ids: modules
                .iter()
                .map(|spec| CapabilityId::from(spec.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: BTreeSet::new(),
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
            declared_services: DeclaredServiceViewDescriptor {
                provided_services: Vec::new(),
                required_service_handles: Vec::new(),
            },
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
    for spec in &modules {
        if spec.actions.is_empty() {
            continue;
        }
        let host_port = action_host_port.as_ref().ok_or_else(|| {
            format!(
                "{} declares an action but no Wave 5 action host port was supplied",
                spec.id
            )
        })?;
        registration
            .add_capability_handler(
                CapabilityId::from(spec.id),
                Arc::new(Wave5CapabilityHandler {
                    capability_id: CapabilityId::from(spec.id),
                    action_ids: spec
                        .actions
                        .iter()
                        .map(|action| ActionId::from(action.id))
                        .collect(),
                    host_port: Arc::clone(host_port),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(registration)
}

fn capability_manifest(
    spec: &ModuleSpec,
    package: &PackageRef,
    declared_port_ids: &BTreeSet<HostPortId>,
) -> CapabilityManifest {
    let actions = spec
        .actions
        .iter()
        .map(|action| CapabilityActionDescriptor {
            action_id: ActionId::from(action.id),
            input_schema: action_schema_ref(
                action.id,
                "input",
                &action_input_schema_for(action.id)
                    .expect("built-in Wave 5 Action has an input schema"),
            ),
            output_schema: action_schema_ref(action.id, "output", &object_schema(true)),
            effect_class: action.effect,
            presentation: nomifun_agent_contracts::ToolPresentationKind::FunctionTool,
        })
        .collect();
    CapabilityManifest {
        id: CapabilityId::from(spec.id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "module:{}",
            spec.id
        )),
        version: VersionString::from(VERSION),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display(spec.display_name, spec.description),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_module_surface_declarations(
            GENERAL_SURFACES.iter().copied(),
            [CapabilityConsumer::Agent],
            CapabilityAuthoringPolicy::Direct,
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: schema_value(),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs: Vec::new(),
            context_phase: Default::default(),
            ui_slot: None,
            event_schema_refs: Vec::new(),
            resource_kinds: spec
                .actions
                .iter()
                .flat_map(|action| action.resource_kinds.iter())
                .map(|kind| ResourceKind::from(*kind))
                .collect(),
            host_ports: [WAVE5_CAPABILITY_HOST_PORT_ID]
                .into_iter()
                .filter(|id| declared_port_ids.contains(&HostPortId::from(*id)))
                .map(host_port)
                .collect(),
        },
    }
}

struct Wave5CapabilityHandler {
    capability_id: CapabilityId,
    action_ids: BTreeSet<ActionId>,
    host_port: Arc<dyn Wave5HostPort>,
}

impl CapabilityHandler for Wave5CapabilityHandler {
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
            let operation = operation_from_input(
                &self.capability_id,
                &context.action_id,
                input,
            )?;
            let request = Wave5HostRequest {
                context: Wave5HostContext {
                    principal: context.principal,
                    agent_session_id: context.agent_session_id,
                    turn_id: context.turn_id,
                    operation_id: context.operation_id,
                    idempotency_key: context.idempotency_key,
                    correlation_id: context.correlation_id,
                    resolved_snapshot_ref: context.resolved_snapshot_ref,
                    registry_generation: context.registry_generation,
                    capability_id: self.capability_id.clone(),
                    action_id: context.action_id,
                    state_scope_key: context.state_scope_key,
                    state: context.state,
                    services: context.services,
                    resource_bindings: context.resource_bindings,
                },
                operation,
            };
            request
                .validate()
                .map_err(|error| host_error_to_kernel(&request.context, error))?;
            let request_context = request.context.clone();
            let result = self
                .host_port
                .invoke(request)
                .await
                .map_err(|error| host_error_to_kernel(&request_context, error))?;
            if !result.0.is_object() {
                return Err(KernelError::capability_execution_failed(
                    WAVE5_INVALID_RESPONSE,
                    format!(
                        "{} host result must be a JSON object",
                        self.capability_id.as_ref()
                    ),
                ));
            }
            Ok(result)
        })
    }
}

/// Convert one exact Module/Action pair and object payload into a typed
/// owner command. Platform-only transports have no match.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    input: StrictJsonValue,
) -> Result<Wave5CapabilityOperation, KernelError> {
    let operation = match (capability_id.as_ref(), action_id.as_ref()) {
        (AGENT_COLLABORATION_MODULE_ID, AGENT_DELEGATE_ACTION_ID) => {
            Wave5CapabilityOperation::AgentDelegate { input }
        }
        (AGENT_COLLABORATION_MODULE_ID, AGENT_FORK_ACTION_ID) => {
            Wave5CapabilityOperation::AgentFork { input }
        }
        (AGENT_COLLABORATION_MODULE_ID, AGENT_REQUEST_USER_DECISION_ACTION_ID) => {
            Wave5CapabilityOperation::AgentRequestUserDecision { input }
        }
        (AUTOMATION_SCHEDULE_MODULE_ID, SCHEDULE_LIST_ACTION_ID) => {
            Wave5CapabilityOperation::ScheduleList { input }
        }
        (AUTOMATION_SCHEDULE_MODULE_ID, SCHEDULE_CREATE_ACTION_ID) => {
            Wave5CapabilityOperation::ScheduleCreate { input }
        }
        (AUTOMATION_SCHEDULE_MODULE_ID, SCHEDULE_UPDATE_ACTION_ID) => {
            Wave5CapabilityOperation::ScheduleUpdate { input }
        }
        (AUTOMATION_SCHEDULE_MODULE_ID, SCHEDULE_DELETE_ACTION_ID) => {
            Wave5CapabilityOperation::ScheduleDelete { input }
        }
        (REQUIREMENTS_MODULE_ID, REQUIREMENTS_READ_ACTION_ID) => {
            Wave5CapabilityOperation::RequirementsRead { input }
        }
        (REQUIREMENTS_MODULE_ID, REQUIREMENTS_WRITE_ACTION_ID) => {
            Wave5CapabilityOperation::RequirementsWrite { input }
        }
        (REQUIREMENTS_MODULE_ID, REQUIREMENTS_STATUS_ACTION_ID) => {
            Wave5CapabilityOperation::RequirementsStatus { input }
        }
        (REQUIREMENTS_MODULE_ID, REQUIREMENTS_CLAIM_ACTION_ID) => {
            Wave5CapabilityOperation::RequirementsClaim { input }
        }
        (module, action) => {
            return Err(KernelError::ActionNotDeclared {
                capability_id: CapabilityId::from(module),
                action_id: ActionId::from(action),
            });
        }
    };
    if !operation.input().0.is_object() {
        return Err(KernelError::capability_execution_failed(
            WAVE5_INVALID_REQUEST,
            format!(
                "{} / {} input must be a JSON object",
                capability_id.as_ref(),
                action_id.as_ref()
            ),
        ));
    }
    Ok(operation)
}

fn host_error_to_kernel(
    context: &Wave5HostContext,
    error: Wave5HostPortError,
) -> KernelError {
    if error.code == nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH {
        if let Some(binding) = context
            .resource_bindings
            .iter()
            .find(|binding| binding.owner_id != context.principal.principal_id)
        {
            return KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            };
        }
    }
    KernelError::capability_execution_failed(error.code, error.message)
}

fn validate_host_context(context: &Wave5HostContext) -> Result<(), Wave5HostPortError> {
    let fields = [
        ("principal.principal_kind", context.principal.principal_kind.as_str()),
        ("principal.principal_id", context.principal.principal_id.as_str()),
        ("agent_session_id", context.agent_session_id.as_ref()),
        ("operation_id", context.operation_id.as_ref()),
        ("idempotency_key", context.idempotency_key.as_ref()),
        ("correlation_id", context.correlation_id.as_ref()),
        (
            "resolved_snapshot_ref.snapshot_id",
            context.resolved_snapshot_ref.snapshot_id.as_ref(),
        ),
        (
            "resolved_snapshot_ref.snapshot_digest",
            context.resolved_snapshot_ref.snapshot_digest.as_ref(),
        ),
        ("state_scope_key", context.state_scope_key.as_ref()),
    ];
    if let Some((field, _)) = fields.iter().find(|(_, value)| value.trim().is_empty()) {
        return Err(Wave5HostPortError::invalid_request(format!(
            "{field} must be non-empty"
        )));
    }
    Ok(())
}

fn validate_resource_bindings_contract(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &[TypedResourceBinding],
) -> Result<(), Wave5HostPortError> {
    if principal_id.trim().is_empty() {
        return Err(Wave5HostPortError::invalid_request(
            "principal.principal_id must be non-empty",
        ));
    }

    let expected_kinds = requirements
        .iter()
        .map(|requirement| ResourceKind::from(requirement.resource_kind))
        .collect::<BTreeSet<_>>();
    let declared_operations = resource_binding_metadata();
    let mut seen_binding_ids = BTreeSet::new();
    let mut seen_resource_kinds = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().trim().is_empty()
            || binding.resource_kind.as_ref().trim().is_empty()
            || binding.resource_id.as_ref().trim().is_empty()
            || binding.owner_id.trim().is_empty()
        {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} requires non-empty binding, resource kind, resource ID, and owner ID",
                capability_id.as_ref()
            )));
        }
        if !seen_binding_ids.insert(binding.binding_id.clone()) {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource binding {}",
                capability_id.as_ref(),
                binding.binding_id.as_ref()
            )));
        }
        if binding.owner_id != principal_id {
            return Err(Wave5HostPortError::new(
                nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH,
                format!(
                    "resource binding {} belongs to {}, not {}",
                    binding.binding_id.as_ref(),
                    binding.owner_id,
                    principal_id
                ),
            ));
        }
        if !seen_resource_kinds.insert(binding.resource_kind.clone()) {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        if !expected_kinds.contains(&binding.resource_kind) {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received unexpected resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        let Some(allowed_operations) = declared_operations.get(&binding.resource_kind) else {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received undeclared resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        };
        if binding.operations.is_empty()
            || binding
                .operations
                .iter()
                .any(|operation| operation.trim().is_empty())
        {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received empty resource operation metadata for {}",
                capability_id.as_ref(),
                binding.binding_id.as_ref()
            )));
        }
        if let Some(operation) = binding
            .operations
            .iter()
            .find(|operation| !allowed_operations.contains(*operation))
        {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} received undeclared operation {} on resource kind {}",
                capability_id.as_ref(),
                operation,
                binding.resource_kind.as_ref()
            )));
        }
    }
    for requirement in requirements {
        let Some(binding) = bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == requirement.resource_kind)
        else {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} is missing resource kind {}",
                capability_id.as_ref(),
                requirement.resource_kind
            )));
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(Wave5HostPortError::resource_binding_invalid(format!(
                "{} requires operation {} on {}",
                capability_id.as_ref(),
                requirement.operation,
                requirement.resource_kind
            )));
        }
    }
    Ok(())
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

/// Canonical model-visible input schema for one exact Action.
/// Schedule schemas mirror `nomifun-cron`'s deny-unknown typed inputs and do
/// not expose platform timer, provider, model or Session implementation fields.
pub fn action_input_schema_for(action_id: &str) -> Result<StrictJsonValue, String> {
    let string = || serde_json::json!({"type": "string", "minLength": 1});
    let optional_string = || {
        serde_json::json!({
            "anyOf": [
                {"type": "string"},
                {"type": "null"}
            ]
        })
    };
    let schema = match action_id {
        SCHEDULE_LIST_ACTION_ID => strict_object_schema(serde_json::json!({}), &[]),
        SCHEDULE_CREATE_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "name": string(),
                "schedule": string(),
                "schedule_description": optional_string(),
                "message": string()
            }),
            &["name", "schedule", "message"],
        ),
        SCHEDULE_UPDATE_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "cron_job_id": string(),
                "name": string(),
                "schedule": string(),
                "schedule_description": optional_string(),
                "message": string()
            }),
            &["cron_job_id", "name", "schedule", "message"],
        ),
        SCHEDULE_DELETE_ACTION_ID => strict_object_schema(
            serde_json::json!({"cron_job_id": string()}),
            &["cron_job_id"],
        ),
        AGENT_DELEGATE_ACTION_ID => StrictJsonValue(serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "strategy": {
                            "type":"string",
                            "const":"planned",
                            "description":"Use the literal planned for one goal that the execution planner should decompose."
                        },
                        "goal": {
                            "type":"string",
                            "minLength":1,
                            "maxLength":65536,
                            "description":"Complete objective for the execution planner."
                        }
                    },
                    "required": ["strategy", "goal"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "strategy": {
                            "type":"string",
                            "const":"parallel",
                            "description":"Use the literal parallel for explicit fan-out."
                        },
                        "tasks": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 16,
                            "description": "Plural tasks: a JSON array of independent upstream Agent tasks. Never send a singular task field.",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "name": {
                                        "type":"string",
                                        "minLength":1,
                                        "description":"Short unique display name for this upstream Agent."
                                    },
                                    "prompt": {
                                        "type":"string",
                                        "minLength":1,
                                        "description":"Complete task prompt for this upstream Agent."
                                    },
                                    "role": {
                                        "type":["string","null"],
                                        "description":"Optional descriptive focus; it never grants tools."
                                    },
                                    "tool_policy": {
                                        "type":"string",
                                        "enum":["full","read_only","read_shell"],
                                        "default":"full"
                                    }
                                },
                                "required": ["name", "prompt"]
                            }
                        },
                        "synthesize": {
                            "type":"boolean",
                            "default":false,
                            "description":"JSON boolean. true creates one downstream read-only Agent that depends on and receives every task result; do not encode it as a string."
                        }
                    },
                    "required": ["strategy", "tasks"]
                }
            ]
        })),
        AGENT_FORK_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "goal": {"type":"string","minLength":1,"maxLength":65536}
            }),
            &["goal"],
        ),
        AGENT_REQUEST_USER_DECISION_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "question": {"type":"string","minLength":1,"maxLength":65536}
            }),
            &["question"],
        ),
        REQUIREMENTS_READ_ACTION_ID => StrictJsonValue(serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "operation":{"const":"get","type":"string"},
                        "requirement_id":{"type":"string","minLength":36,"maxLength":36}
                    },
                    "required":["operation","requirement_id"]
                },
                {
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "operation":{"const":"list","type":"string"},
                        "tag":{"type":["string","null"],"maxLength":256},
                        "status":{"type":["string","null"],"enum":["pending","in_progress","done","failed","cancelled","needs_review",null]},
                        "query":{"type":["string","null"],"maxLength":4096},
                        "page":{"type":["integer","null"],"minimum":1},
                        "page_size":{"type":["integer","null"],"minimum":1,"maximum":200}
                    },
                    "required":["operation"]
                }
            ]
        })),
        REQUIREMENTS_WRITE_ACTION_ID => StrictJsonValue(serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "operation":{"const":"create","type":"string"},
                        "title":{"type":"string","minLength":1,"maxLength":2048},
                        "content":{"type":["string","null"],"maxLength":65536},
                        "tag":{"type":"string","minLength":1,"maxLength":256}
                    },
                    "required":["operation","title","tag"]
                },
                {
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "operation":{"const":"update","type":"string"},
                        "requirement_id":{"type":"string","minLength":36,"maxLength":36},
                        "title":{"type":["string","null"],"maxLength":2048},
                        "content":{"type":["string","null"],"maxLength":65536},
                        "tag":{"type":["string","null"],"maxLength":256}
                    },
                    "required":["operation","requirement_id"]
                },
                {
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "operation":{"const":"delete","type":"string"},
                        "requirement_id":{"type":"string","minLength":36,"maxLength":36}
                    },
                    "required":["operation","requirement_id"]
                }
            ]
        })),
        REQUIREMENTS_STATUS_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "requirement_id":{"type":"string","minLength":36,"maxLength":36},
                "status":{"type":"string","enum":["pending","done","failed","cancelled","needs_review"]},
                "completion_note":{"type":["string","null"],"maxLength":65536}
            }),
            &["requirement_id","status"],
        ),
        REQUIREMENTS_CLAIM_ACTION_ID => strict_object_schema(
            serde_json::json!({
                "tag":{"type":"string","minLength":1,"maxLength":256}
            }),
            &["tag"],
        ),
        _ => return Err(format!("unknown Wave 5 Action {action_id}")),
    };
    Ok(schema)
}

pub fn resolve_action_schema(
    module_id: &str,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    let module = module_spec(module_id)
        .ok_or_else(|| format!("unknown Wave 5 Module {module_id}"))?;
    for action in module.actions {
        let input = action_input_schema_for(action.id)?;
        if action_schema_ref(action.id, "input", &input) == *reference {
            return Ok(input);
        }
        let output = object_schema(true);
        if action_schema_ref(action.id, "output", &output) == *reference {
            return Ok(output);
        }
    }
    Err(format!(
        "schema {} is not owned by Wave 5 Module {module_id}",
        reference.as_ref()
    ))
}

fn strict_object_schema(
    properties: serde_json::Value,
    required: &[&str],
) -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    }))
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
    value
}

fn empty_object() -> StrictJsonValue {
    StrictJsonValue(std::iter::empty::<(String, String)>().collect())
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

fn action_schema_ref(
    subject: &str,
    role: &str,
    schema: &StrictJsonValue,
) -> CanonicalSchemaRef {
    let digest = nomifun_agent_contracts::digest_payload(schema)
        .expect("the built-in Action schema is canonicalizable");
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

    fn action_ids(capability: &CapabilityManifest) -> BTreeSet<String> {
        capability
            .contributions
            .actions
            .iter()
            .map(|action| action.action_id.as_ref().to_owned())
            .collect()
    }

    #[test]
    fn catalog_contains_only_three_product_modules() {
        let registrations = registrations().expect("Wave 5 registrations");
        assert_eq!(registrations.len(), 4);
        let capabilities = registrations
            .iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
            })
            .collect::<Vec<_>>();
        assert_eq!(capabilities.len(), 3);
        assert_eq!(
            capabilities
                .iter()
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>(),
            target_capability_ids()
        );
        assert!(capabilities.iter().all(|capability| {
            capability.kind == CapabilityKind::Tool
                && capability.authoring_policy() == Ok(CapabilityAuthoringPolicy::Direct)
        }));

        let by_id = capabilities
            .into_iter()
            .map(|capability| (capability.id.as_ref(), capability))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            action_ids(by_id[AGENT_COLLABORATION_MODULE_ID]),
            BTreeSet::from([
                AGENT_DELEGATE_ACTION_ID.to_owned(),
                AGENT_FORK_ACTION_ID.to_owned(),
                AGENT_REQUEST_USER_DECISION_ACTION_ID.to_owned(),
            ])
        );
        assert_eq!(
            action_ids(by_id[AUTOMATION_SCHEDULE_MODULE_ID]),
            BTreeSet::from([
                SCHEDULE_LIST_ACTION_ID.to_owned(),
                SCHEDULE_CREATE_ACTION_ID.to_owned(),
                SCHEDULE_UPDATE_ACTION_ID.to_owned(),
                SCHEDULE_DELETE_ACTION_ID.to_owned(),
            ])
        );
        assert_eq!(
            action_ids(by_id[REQUIREMENTS_MODULE_ID]),
            BTreeSet::from([
                REQUIREMENTS_READ_ACTION_ID.to_owned(),
                REQUIREMENTS_WRITE_ACTION_ID.to_owned(),
                REQUIREMENTS_STATUS_ACTION_ID.to_owned(),
                REQUIREMENTS_CLAIM_ACTION_ID.to_owned(),
            ])
        );
    }

    #[test]
    fn platform_controllers_and_remote_transports_are_not_agent_grants() {
        let remote = remote_ingress_registration().expect("remote transport registration");
        assert!(
            remote
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .is_empty()
        );
        assert!(remote_transport_descriptor().is_exact_contract());
        assert!(remote_admission_descriptor().is_exact_contract());
        assert!(remote_drain_descriptor().is_exact_contract());
    }

    #[test]
    fn exact_actions_map_to_one_typed_owner_operation() {
        let cases = [
            (
                AGENT_COLLABORATION_MODULE_ID,
                AGENT_DELEGATE_ACTION_ID,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_COLLABORATION_MODULE_ID,
                AGENT_FORK_ACTION_ID,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_COLLABORATION_MODULE_ID,
                AGENT_REQUEST_USER_DECISION_ACTION_ID,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_LIST_ACTION_ID,
                Wave5OwnerDomain::Schedule,
            ),
            (
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_CREATE_ACTION_ID,
                Wave5OwnerDomain::Schedule,
            ),
            (
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_UPDATE_ACTION_ID,
                Wave5OwnerDomain::Schedule,
            ),
            (
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_DELETE_ACTION_ID,
                Wave5OwnerDomain::Schedule,
            ),
            (
                REQUIREMENTS_MODULE_ID,
                REQUIREMENTS_READ_ACTION_ID,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_MODULE_ID,
                REQUIREMENTS_WRITE_ACTION_ID,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_MODULE_ID,
                REQUIREMENTS_STATUS_ACTION_ID,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_MODULE_ID,
                REQUIREMENTS_CLAIM_ACTION_ID,
                Wave5OwnerDomain::Requirements,
            ),
        ];
        for (module, action, owner) in cases {
            let operation = operation_from_input(
                &CapabilityId::from(module),
                &ActionId::from(action),
                empty_object(),
            )
            .expect("known exact action");
            assert_eq!(operation.capability_id(), CapabilityId::from(module));
            assert_eq!(operation.action_id(), ActionId::from(action));
            assert_eq!(operation.owner_domain(), owner);
        }

        assert!(
            operation_from_input(
                &CapabilityId::from(REQUIREMENTS_MODULE_ID),
                &ActionId::from(AGENT_DELEGATE_ACTION_ID),
                empty_object(),
            )
            .is_err()
        );
    }

    #[test]
    fn action_resources_are_exact_and_do_not_widen_siblings() {
        assert_eq!(
            action_spec(
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_LIST_ACTION_ID,
            )
            .unwrap()
            .requirements[0]
            .operation,
            "read"
        );
        assert_eq!(
            action_spec(
                AUTOMATION_SCHEDULE_MODULE_ID,
                SCHEDULE_DELETE_ACTION_ID,
            )
            .unwrap()
            .requirements[0]
            .operation,
            "delete"
        );
        assert!(
            action_spec(AGENT_COLLABORATION_MODULE_ID, AGENT_FORK_ACTION_ID)
                .unwrap()
                .requirements
                .is_empty()
        );
        assert!(
            action_spec(AGENT_COLLABORATION_MODULE_ID, AGENT_DELEGATE_ACTION_ID)
                .unwrap()
                .requirements
                .is_empty()
        );
        assert!(
            action_spec(
                AGENT_COLLABORATION_MODULE_ID,
                AGENT_REQUEST_USER_DECISION_ACTION_ID,
            )
            .unwrap()
            .requirements
            .is_empty()
        );
    }

    #[test]
    fn schedule_action_schemas_are_closed_and_owner_typed() {
        let list = action_input_schema_for(SCHEDULE_LIST_ACTION_ID).unwrap();
        assert_eq!(list.0["additionalProperties"], false);
        assert_eq!(list.0["required"], serde_json::json!([]));

        let create = action_input_schema_for(SCHEDULE_CREATE_ACTION_ID).unwrap();
        assert_eq!(create.0["additionalProperties"], false);
        assert_eq!(
            create.0["required"],
            serde_json::json!(["name", "schedule", "message"])
        );
        let properties = create.0["properties"].as_object().unwrap();
        assert_eq!(
            properties.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "message".to_owned(),
                "name".to_owned(),
                "schedule".to_owned(),
                "schedule_description".to_owned(),
            ])
        );
        for forbidden in ["provider_id", "model", "session_id", "timer"] {
            assert!(!properties.contains_key(forbidden));
        }

        let update = action_input_schema_for(SCHEDULE_UPDATE_ACTION_ID).unwrap();
        assert!(update.0["properties"].get("cron_job_id").is_some());
        let delete = action_input_schema_for(SCHEDULE_DELETE_ACTION_ID).unwrap();
        assert_eq!(
            delete.0["required"],
            serde_json::json!(["cron_job_id"])
        );
    }

    #[test]
    fn decision_action_schema_accepts_only_the_question() {
        let schema = action_input_schema_for(AGENT_REQUEST_USER_DECISION_ACTION_ID).unwrap();
        assert_eq!(schema.0["additionalProperties"], false);
        assert_eq!(schema.0["required"], serde_json::json!(["question"]));
        assert_eq!(
            schema.0["properties"].as_object().unwrap().keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from(["question".to_owned()])
        );
    }

    #[test]
    fn delegate_schema_accepts_one_parallel_fanout_with_optional_synthesis() {
        let schema = action_input_schema_for(AGENT_DELEGATE_ACTION_ID).unwrap();
        let validator = jsonschema::options()
            .build(&schema.0)
            .expect("agent/delegate schema must compile");

        assert!(validator.is_valid(&serde_json::json!({
            "strategy": "planned",
            "goal": "plan a dependency graph"
        })));
        assert!(validator.is_valid(&serde_json::json!({
            "strategy": "parallel",
            "tasks": [
                {"name": "upstream-a", "prompt": "produce A"},
                {
                    "name": "upstream-b",
                    "prompt": "produce B",
                    "role": "critic",
                    "tool_policy": "read_only"
                }
            ],
            "synthesize": true
        })));
        for invalid in [
            serde_json::json!({"goal":"new Snapshots require an explicit strategy"}),
            serde_json::json!({"strategy":"parallel","tasks":[]}),
            serde_json::json!({
                "strategy":"parallel",
                "tasks":[{"name":"upstream","prompt":"work"}],
                "goal":"mixed variants are forbidden"
            }),
            serde_json::json!({
                "strategy":"parallel",
                "tasks":[{"name":"upstream","prompt":"work","tool_policy":"admin"}]
            }),
        ] {
            assert!(!validator.is_valid(&invalid), "unexpectedly accepted {invalid}");
        }

        let fork = action_input_schema_for(AGENT_FORK_ACTION_ID).unwrap();
        let fork_validator = jsonschema::options()
            .build(&fork.0)
            .expect("agent/fork schema must compile");
        assert!(!fork_validator.is_valid(&serde_json::json!({
            "strategy":"parallel",
            "tasks":[{"name":"upstream","prompt":"work"}]
        })));
    }

    #[test]
    fn requirements_union_schemas_have_a_strict_model_tool_object_root() {
        for action in [REQUIREMENTS_READ_ACTION_ID, REQUIREMENTS_WRITE_ACTION_ID] {
            let schema = action_input_schema_for(action).unwrap();
            assert_eq!(schema.0["type"], "object", "{action}");
            let variants = schema.0["oneOf"].as_array().unwrap();
            assert!(!variants.is_empty() && variants.len() <= 16, "{action}");
            for variant in variants {
                assert_eq!(variant["type"], "object", "{action}");
                assert_eq!(variant["additionalProperties"], false, "{action}");
                assert!(variant["properties"].is_object(), "{action}");
            }
        }
    }
}
