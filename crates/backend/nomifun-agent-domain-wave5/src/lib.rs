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
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
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
    ServiceHandleDescriptor, ServiceKeyRef, ServiceRequirement, StrictJsonValue,
    TypedCommandPortDescriptor, TypedResourceBinding, TypedResourceBindings, RuntimeTarget,
    VersionString, REMOTE_AUTH_REQUIRED, agent_core_mount_id, agent_core_package_ref,
    agent_session_command_service_ref, agent_session_query_service_ref,
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
pub const AUTOWORK_SCHEDULER_PACKAGE: &str = "nomifun.autowork-scheduler";
pub const IDMM_PACKAGE: &str = "nomifun.idmm";
pub const REMOTE_INGRESS_PACKAGE: &str = "nomifun.remote-ingress";
pub const REQUIREMENTS_PACKAGE: &str = "nomifun.requirements";

pub const AGENT_EXECUTION_PACKAGE_ID: &str = AGENT_EXECUTION_PACKAGE;
pub const AUTOWORK_SCHEDULER_PACKAGE_ID: &str = AUTOWORK_SCHEDULER_PACKAGE;
pub const IDMM_PACKAGE_ID: &str = IDMM_PACKAGE;
pub const REMOTE_INGRESS_PACKAGE_ID: &str = REMOTE_INGRESS_PACKAGE;
pub const REQUIREMENTS_PACKAGE_ID: &str = REQUIREMENTS_PACKAGE;
pub const REMOTE_INGRESS_MOUNT_ID: &str = "nomifun-remote-ingress";

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

/// Resource kind used by AgentExecution capabilities that operate on a
/// session-backed process/PTY lane.
pub const PROCESS_SESSION_RESOURCE_KIND: &str = "process_session";

const PROCESS_SESSION_RESOURCES: &[&str] = &[PROCESS_SESSION_RESOURCE_KIND];

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

/// The single host port for action-bearing Wave 5 capabilities.
///
/// Wave 5 owns the capability vocabulary and input boundary, while the
/// application owns AgentExecution, scheduling, IDMM, and requirements facts.
/// Keeping this port in the domain crate avoids a dependency on the
/// application composition root and prevents a synthetic success result when
/// no owner has been wired.
pub const WAVE5_CAPABILITY_HOST_PORT_ID: &str = "host.wave5.capability.invoke";
pub const WAVE5_HOST_PORT_UNAVAILABLE: &str = "WAVE5_HOST_PORT_UNAVAILABLE";
pub const WAVE5_INVALID_REQUEST: &str = "WAVE5_INVALID_REQUEST";
pub const WAVE5_ACTION_OPERATION_MISMATCH: &str = "WAVE5_ACTION_OPERATION_MISMATCH";
pub const WAVE5_RESOURCE_BINDING_INVALID: &str = "WAVE5_RESOURCE_BINDING_INVALID";

/// Kernel-authorized invocation context projected to the application owner.
///
/// `state` is the already namespace-scoped Kernel handle for this package
/// mount. It is intentionally the only state surface exposed here: no raw
/// persistence, registry, database, or service bag crosses the boundary.
#[derive(Clone)]
pub struct Wave5HostContext {
    pub principal: nomifun_agent_contracts::PrincipalRef,
    pub agent_session_id: AgentSessionId,
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
    AgentExecutionPlan { input: StrictJsonValue },
    AgentExecutionSteer { input: StrictJsonValue },
    AgentExecutionObserve { input: StrictJsonValue },
    ScheduleStore { input: StrictJsonValue },
    RequirementsRead { input: StrictJsonValue },
    RequirementsWrite { input: StrictJsonValue },
    RequirementsStatus { input: StrictJsonValue },
    RequirementsClaim { input: StrictJsonValue },
}

impl Wave5CapabilityOperation {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::AgentDelegate { .. } => AGENT_DELEGATE,
            Self::AgentFork { .. } => AGENT_FORK,
            Self::AgentExecutionPlan { .. } => AGENT_EXECUTION_PLAN,
            Self::AgentExecutionSteer { .. } => AGENT_EXECUTION_STEER,
            Self::AgentExecutionObserve { .. } => AGENT_EXECUTION_OBSERVE,
            Self::ScheduleStore { .. } => SCHEDULE_STORE,
            Self::RequirementsRead { .. } => REQUIREMENTS_READ,
            Self::RequirementsWrite { .. } => REQUIREMENTS_WRITE,
            Self::RequirementsStatus { .. } => REQUIREMENTS_STATUS,
            Self::RequirementsClaim { .. } => REQUIREMENTS_CLAIM,
        })
    }

    pub fn action_id(&self) -> ActionId {
        action_id(self.capability_id().as_ref())
            .expect("every Wave 5 action operation has a canonical action")
    }

    pub fn owner_domain(&self) -> Wave5OwnerDomain {
        match self {
            Self::AgentDelegate { .. }
            | Self::AgentFork { .. }
            | Self::AgentExecutionPlan { .. }
            | Self::AgentExecutionSteer { .. }
            | Self::AgentExecutionObserve { .. } => Wave5OwnerDomain::AgentExecution,
            Self::ScheduleStore { .. } => Wave5OwnerDomain::Schedule,
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
            | Self::AgentExecutionPlan { input }
            | Self::AgentExecutionSteer { input }
            | Self::AgentExecutionObserve { input }
            | Self::ScheduleStore { input }
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
        let Some(spec) = capability_spec(capability_id.as_ref()) else {
            return Err(Wave5HostPortError::invalid_request(format!(
                "unknown Wave 5 capability {}",
                capability_id.as_ref()
            )));
        };
        if spec.actions.is_empty() {
            return Err(Wave5HostPortError::action_operation_mismatch(format!(
                "{} is transport/scheduler/middleware owned and has no action host operation",
                capability_id.as_ref()
            )));
        }
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
            spec.requirements,
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
            && self.transport_port.id.as_ref() == REMOTE_TRANSPORT_PORT
            && self.admission_port.id.as_ref() == REMOTE_ADMISSION_PORT
            && self.drain_port.id.as_ref() == REMOTE_DRAIN_PORT
            && command_port_ids
                == BTreeSet::from([
                    REMOTE_OPEN_PORT,
                    REMOTE_TURN_PORT,
                    REMOTE_OBSERVE_PORT,
                    REMOTE_CANCEL_PORT,
                ])
            && self.typed_command_ports.len() == REMOTE_OPERATION_IDS.len()
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

/// Return the five bundled registrations owned by Wave 5.
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
            agent_execution_capabilities(),
            agent_execution_ports(),
            Some(Arc::clone(&action_host_port)),
        )?,
        registration_for(
            AUTOWORK_SCHEDULER_PACKAGE,
            "nomifun-autowork-scheduler",
            autowork_capabilities(),
            autowork_ports(),
            Some(Arc::clone(&action_host_port)),
        )?,
        registration_for(
            IDMM_PACKAGE,
            "nomifun-idmm",
            idmm_capabilities(),
            idmm_ports(),
            None,
        )?,
        registration_for(
            REMOTE_INGRESS_PACKAGE,
            REMOTE_INGRESS_MOUNT_ID,
            remote_capabilities(),
            remote_ports(),
            None,
        )?,
        registration_for(
            REQUIREMENTS_PACKAGE,
            "nomifun-requirements",
            requirements_capabilities(),
            requirements_ports(),
            Some(action_host_port),
        )?,
    ])
}

pub fn agent_execution_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AGENT_EXECUTION_PACKAGE,
        "nomifun-agent-execution",
        agent_execution_capabilities(),
        agent_execution_ports(),
        Some(unconfigured_host_port()),
    )
}

pub fn autowork_registration() -> Result<PluginRegistration, String> {
    registration_for(
        AUTOWORK_SCHEDULER_PACKAGE,
        "nomifun-autowork-scheduler",
        autowork_capabilities(),
        autowork_ports(),
        Some(unconfigured_host_port()),
    )
}

pub fn idmm_registration() -> Result<PluginRegistration, String> {
    registration_for(
        IDMM_PACKAGE,
        "nomifun-idmm",
        idmm_capabilities(),
        idmm_ports(),
        None,
    )
}

pub fn remote_ingress_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REMOTE_INGRESS_PACKAGE,
        REMOTE_INGRESS_MOUNT_ID,
        remote_capabilities(),
        remote_ports(),
        None,
    )
}

pub fn requirements_registration() -> Result<PluginRegistration, String> {
    registration_for(
        REQUIREMENTS_PACKAGE,
        "nomifun-requirements",
        requirements_capabilities(),
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
        slot_key: PROCESS_SESSION_RESOURCE_KIND,
        resource_kind: ResourceKind::from(PROCESS_SESSION_RESOURCE_KIND),
        required: false,
        operations: BTreeSet::from(["execute".to_owned(), "observe".to_owned()]),
        binding_policy: "leave_unbound",
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
    vec![typed_resource_binding(
        "wave5-process-session",
        PROCESS_SESSION_RESOURCE_KIND,
        "wave5-process-session",
        owner_id,
        ["execute", "observe"],
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

pub fn typed_resource_bindings_for<'a>(
    owner_id: &str,
    entries: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a [&'a str])>,
) -> TypedResourceBindings {
    entries
        .into_iter()
        .map(
            |(binding_id, resource_kind, resource_id, operations)| {
                typed_resource_binding(
                    binding_id,
                    resource_kind,
                    resource_id,
                    owner_id,
                    operations.iter().copied(),
                )
            },
        )
        .collect()
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
    capability_spec(id).map(|spec| {
        spec.resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
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

#[derive(Clone, Copy)]
struct CapabilitySpec {
    id: &'static str,
    kind: CapabilityKind,
    effect: EffectClass,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
    surfaces: &'static [&'static str],
    host_ports: &'static [&'static str],
    actions: &'static [&'static str],
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
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID, "agent-execution.dispatch"],
    command_ports: &["agent-execution.session-command"],
    outbox_ports: &["agent-execution.outbox"],
};

const AUTOWORK_PORTS: PortSpec = PortSpec {
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID, "autowork.scheduler"],
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
    host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID, "requirements.board"],
    command_ports: &["requirements.command"],
    outbox_ports: &["requirements.outbox"],
};

const fn tool_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Tool,
        effect: EffectClass::WriteReversible,
        resource_kinds: &[],
        requirements: &[],
        surfaces: GENERAL_SURFACES,
        host_ports: &[WAVE5_CAPABILITY_HOST_PORT_ID],
        actions: &[],
    }
}

const fn scheduler_spec(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Scheduler,
        effect: EffectClass::WriteDurable,
        resource_kinds: &[],
        requirements: &[],
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
        resource_kinds: &[],
        requirements: &[],
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
        resource_kinds: &[],
        requirements: &[],
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
            effect: EffectClass::ExecuteLocal,
            resource_kinds: PROCESS_SESSION_RESOURCES,
            requirements: &[ResourceRequirement {
                resource_kind: PROCESS_SESSION_RESOURCE_KIND,
                operation: "execute",
            }],
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
            effect: EffectClass::WriteDurable,
            resource_kinds: PROCESS_SESSION_RESOURCES,
            requirements: &[ResourceRequirement {
                resource_kind: PROCESS_SESSION_RESOURCE_KIND,
                operation: "execute",
            }],
            ..tool_spec(AGENT_EXECUTION_STEER)
        },
        CapabilitySpec {
            actions: &[AGENT_EXECUTION_OBSERVE_ACTION],
            effect: EffectClass::ReadLocal,
            resource_kinds: PROCESS_SESSION_RESOURCES,
            requirements: &[ResourceRequirement {
                resource_kind: PROCESS_SESSION_RESOURCE_KIND,
                operation: "observe",
            }],
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
            effect: EffectClass::ReadSensitive,
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

fn capability_spec(id: &str) -> Option<CapabilitySpec> {
    [
        agent_execution_capabilities(),
        autowork_capabilities(),
        idmm_capabilities(),
        remote_capabilities(),
        requirements_capabilities(),
    ]
    .into_iter()
    .flatten()
    .find(|spec| spec.id == id)
}

fn registration_for(
    package_id: &'static str,
    mount_id: &'static str,
    capabilities: Vec<CapabilitySpec>,
    ports: PortSpec,
    action_host_port: Option<Arc<dyn Wave5HostPort>>,
) -> Result<PluginRegistration, String> {
    let package = package_ref(package_id);
    let required_service_refs = required_agent_session_services(package_id);
    let required_service_handles = required_service_refs
        .iter()
        .cloned()
        .map(|service| ServiceHandleDescriptor {
            service,
            provider_package: agent_core_package_ref(),
            provider_mount_id: agent_core_mount_id(),
        })
        .collect();
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
        requires_services: required_service_refs
            .iter()
            .cloned()
            .map(|service| ServiceRequirement { service })
            .collect(),
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
            declared_services: DeclaredServiceViewDescriptor {
                provided_services: Vec::new(),
                required_service_handles,
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
    for spec in &capabilities {
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
                        .map(|action| ActionId::from(*action))
                        .collect(),
                    host_port: Arc::clone(host_port),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(registration)
}

fn required_agent_session_services(package_id: &str) -> Vec<ServiceKeyRef> {
    match package_id {
        AGENT_EXECUTION_PACKAGE | AUTOWORK_SCHEDULER_PACKAGE => {
            vec![agent_session_command_service_ref()]
        }
        IDMM_PACKAGE => vec![agent_session_query_service_ref()],
        REMOTE_INGRESS_PACKAGE => vec![
            agent_session_command_service_ref(),
            agent_session_query_service_ref(),
        ],
        REQUIREMENTS_PACKAGE => Vec::new(),
        _ => Vec::new(),
    }
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
        display: display(spec.id, "Wave 5 capability contribution."),
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
            resource_kinds: spec
                .resource_kinds
                .iter()
                .map(|kind| ResourceKind::from(*kind))
                .collect(),
            host_ports: spec
                .host_ports
                .iter()
                .filter(|id| declared_port_ids.contains(&HostPortId::from(**id)))
                .map(|id| host_port(*id).clone())
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
            let request = Wave5HostRequest {
                context: Wave5HostContext {
                    principal: context.principal,
                    agent_session_id: context.agent_session_id,
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
                operation: operation_from_input(&self.capability_id, input)?,
            };
            request
                .validate()
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?;
            let request_context = request.context.clone();
            self.host_port
                .invoke(request)
                .await
                .map_err(|error| host_error_to_kernel(&request_context, error))
        })
    }
}

/// Convert a canonical capability ID and object payload into one exact typed
/// action operation. Remote capability IDs intentionally have no match.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave5CapabilityOperation, KernelError> {
    let operation = match capability_id.as_ref() {
        AGENT_DELEGATE => Wave5CapabilityOperation::AgentDelegate { input },
        AGENT_FORK => Wave5CapabilityOperation::AgentFork { input },
        AGENT_EXECUTION_PLAN => Wave5CapabilityOperation::AgentExecutionPlan { input },
        AGENT_EXECUTION_STEER => Wave5CapabilityOperation::AgentExecutionSteer { input },
        AGENT_EXECUTION_OBSERVE => Wave5CapabilityOperation::AgentExecutionObserve { input },
        SCHEDULE_STORE => Wave5CapabilityOperation::ScheduleStore { input },
        REQUIREMENTS_READ => Wave5CapabilityOperation::RequirementsRead { input },
        REQUIREMENTS_WRITE => Wave5CapabilityOperation::RequirementsWrite { input },
        REQUIREMENTS_STATUS => Wave5CapabilityOperation::RequirementsStatus { input },
        REQUIREMENTS_CLAIM => Wave5CapabilityOperation::RequirementsClaim { input },
        other => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{other} does not expose an action host operation"),
            });
        }
    };
    Ok(operation)
}

fn host_error_to_kernel(
    context: &Wave5HostContext,
    error: Wave5HostPortError,
) -> KernelError {
    if error.code == nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH {
        let binding_id = context
            .resource_bindings
            .iter()
            .find(|binding| binding.owner_id != context.principal.principal_id)
            .map(|binding| binding.binding_id.clone())
            .unwrap_or_else(|| ResourceBindingId::from("unknown"));
        return KernelError::ResourceOwnerMismatch { binding_id };
    }
    KernelError::CapabilityExecution {
        reason: error.to_string(),
    }
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
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, CapabilityExposure,
        CapabilityRef, CapabilitySelection, DigestHex, PresetRevisionRef, PrincipalRef,
        RuntimeProfileKind, StateKey, UserId,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest, CompilerEnvironment,
        HostPluginStateApi, InMemoryPluginStatePersistence, KernelRegistry,
        MaterializationPolicy, ServiceKey, SessionCapabilityState,
    };

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave5-test-owner".to_owned(),
        }
    }

    trait TestAgentSessionService: Send + Sync {}

    struct TestAgentSessionServiceImpl;

    impl TestAgentSessionService for TestAgentSessionServiceImpl {}

    fn test_agent_core_registration() -> PluginRegistration {
        let package = agent_core_package_ref();
        let mount_id = agent_core_mount_id();
        let command_ref = agent_session_command_service_ref();
        let query_ref = agent_session_query_service_ref();
        let config_schema = schema_value();
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::Bundled,
            source_identity: package.id.as_ref().to_owned(),
            source_digest: None,
        };
        let identity = PluginIdentityDescriptor {
            package: package.clone(),
            mount_id: mount_id.clone(),
        };
        let cancellation_port = host_port("host.plugin.cancel");
        let task_port = host_port("host.plugin.tasks");
        let manifest = PackageManifest {
            schema_version: VersionString::from(VERSION),
            host_contract_version: VersionString::from(VERSION),
            package_id: package.id.clone(),
            package_version: package.version.clone(),
            display: display(
                "Test Agent Session Core",
                "Test-only provider for Wave 5 ServiceKey materialization.",
            ),
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: config_schema.clone(),
            provides_services: vec![
                nomifun_agent_contracts::ServiceProvision {
                    service: command_ref.clone(),
                },
                nomifun_agent_contracts::ServiceProvision {
                    service: query_ref.clone(),
                },
            ],
            requires_services: Vec::new(),
            entrypoint: InProcessEntrypointMetadata {
                entrypoint_profile: "trusted-in-process".to_owned(),
                entrypoint_id: "platform.agent-core.test".to_owned(),
                contract_version: VersionString::from(VERSION),
            },
            contributions: PackageContributions::default(),
        };
        let metadata = PluginRegistrationMetadata {
            manifest: ArtifactEnvelope::new(manifest).expect("test provider manifest"),
            mount_id: mount_id.clone(),
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
                    PluginRegistrarOperation::ProvideService,
                    PluginRegistrarOperation::BindHostPort,
                ]),
                declared_capability_ids: BTreeSet::new(),
                declared_skill_ids: BTreeSet::new(),
                declared_mcp_tool_keys: BTreeSet::new(),
                declared_service_keys: BTreeSet::from([
                    command_ref.id.clone(),
                    query_ref.id.clone(),
                ]),
                declared_host_ports: BTreeSet::from([
                    cancellation_port.id.clone(),
                    task_port.id.clone(),
                ]),
            },
            context: PluginContextDescriptor {
                identity,
                source,
                validated_config: nomifun_agent_contracts::ValidatedPluginConfig {
                    schema_digest: nomifun_agent_contracts::digest_payload(&config_schema)
                        .expect("test provider config digest"),
                    config_revision: 1,
                    value: empty_object(),
                },
                state: PluginStateHandleDescriptor {
                    package_id: package.id,
                    mount_id: mount_id.clone(),
                    methods: PluginStateMethod::REQUIRED.into_iter().collect(),
                },
                declared_services: DeclaredServiceViewDescriptor {
                    provided_services: vec![command_ref.clone(), query_ref.clone()],
                    required_service_handles: Vec::new(),
                },
                host_ports: Vec::new(),
                typed_command_ports: Vec::new(),
                domain_outbox_ports: Vec::new(),
                cancellation: CancellationDescriptor {
                    cancellation_port,
                    scope_key: ScopeKey::from(format!("mount:{}", mount_id.as_ref())),
                },
                managed_task_registration: ManagedTaskRegistrationDescriptor {
                    registrar_port: task_port,
                    scope_key: ScopeKey::from(format!("mount:{}", mount_id.as_ref())),
                },
            },
        };
        let command_key =
            ServiceKey::<dyn TestAgentSessionService>::from_ref(command_ref);
        let query_key =
            ServiceKey::<dyn TestAgentSessionService>::from_ref(query_ref);
        let service =
            Arc::new(TestAgentSessionServiceImpl) as Arc<dyn TestAgentSessionService>;
        let mut registration = PluginRegistration::new(metadata);
        registration
            .provide_service(&command_key, Arc::clone(&service))
            .expect("test command service");
        registration
            .provide_service(&query_key, service)
            .expect("test query service");
        registration
    }

    fn materializable_registrations(
        host: Arc<dyn Wave5HostPort>,
    ) -> Vec<PluginRegistration> {
        let mut registrations = vec![test_agent_core_registration()];
        registrations.extend(
            registrations_with_host_port(host)
                .expect("Wave 5 registrations should build"),
        );
        registrations
    }

    fn compiled_schedule(
        registry: &KernelRegistry,
        owner: &PrincipalRef,
    ) -> (
        nomifun_agent_kernel::CompiledSnapshot,
        nomifun_agent_kernel::ActiveCapabilitySetSnapshot,
    ) {
        let materialized = registry.snapshot().expect("registry snapshot");
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from(SCHEDULE_STORE),
                    version: VersionString::from(VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([ActionId::from(SCHEDULE_STORE_ACTION)]),
                resource_binding_refs: Vec::new(),
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: StrictJsonValue(serde_json::json!({})),
            }],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            persona: "Wave 5 test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            context_policy: StrictJsonValue(serde_json::json!({})),
            execution_constraints: StrictJsonValue(serde_json::json!({})),
            runtime_budget: StrictJsonValue(serde_json::json!({})),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave5-test"),
                revision: 1,
                revision_digest: nomifun_agent_contracts::digest_payload(&payload)
                    .expect("revision digest"),
            },
            payload,
            created_by: UserId::from(owner.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(VERSION),
                required_runtime_protocol_version: VersionString::from(VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("windows-desktop-x64"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave5-test".to_owned(),
            },
            CompileRequest {
                revision,
                principal: owner.clone(),
                scene: "wave5-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave5-resolve"),
            },
        )
        .expect("compile schedule.store");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        (snapshot, active)
    }

    struct StateBackedHost;

    impl Wave5HostPort for StateBackedHost {
        fn invoke<'a>(
            &'a self,
            request: Wave5HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>>
        {
            Box::pin(async move {
                request.validate()?;
                let services = request.context.services.descriptors();
                let state = request.context.state;
                let scope = request.context.state_scope_key;
                let key = StateKey::from("wave5-test-state");
                let entry = state
                    .get(&scope, &key)
                    .await
                    .map_err(|error| Wave5HostPortError::new("STATE_READ_FAILED", error.to_string()))?;
                let descriptor = state.descriptor();
                Ok(StrictJsonValue(serde_json::json!({
                    "package_id": descriptor.package_id,
                    "mount_id": descriptor.mount_id,
                    "state_scope": scope,
                    "state_key": key,
                    "present": entry.is_some(),
                    "service_ids": services
                        .iter()
                        .map(|service| service.service.id.as_ref())
                        .collect::<Vec<_>>(),
                    "service_provider_mounts": services
                        .iter()
                        .map(|service| service.provider_mount_id.as_ref())
                        .collect::<Vec<_>>(),
                })))
            })
        }
    }

    struct CanonicalErrorHost;

    impl Wave5HostPort for CanonicalErrorHost {
        fn invoke<'a>(
            &'a self,
            request: Wave5HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>>
        {
            Box::pin(async move {
                request.validate()?;
                Err(Wave5HostPortError::new(
                    REMOTE_AUTH_REQUIRED,
                    "request-admission fence is closed",
                ))
            })
        }
    }

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
                    assert!(capability
                        .contributions
                        .host_ports
                        .iter()
                        .any(|port| port.id.as_ref() == WAVE5_CAPABILITY_HOST_PORT_ID));
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(!registration.handler_ids().contains(&capability.id));
                    assert!(!capability
                        .contributions
                        .host_ports
                        .iter()
                        .any(|port| port.id.as_ref() == WAVE5_CAPABILITY_HOST_PORT_ID));
                }
            }
        }
    }

    #[test]
    fn agent_session_service_dependencies_match_the_target_map() {
        let registrations = registrations().expect("Wave 5 registrations");
        let expected = [
            (
                AGENT_EXECUTION_PACKAGE,
                vec![agent_session_command_service_ref()],
            ),
            (
                AUTOWORK_SCHEDULER_PACKAGE,
                vec![agent_session_command_service_ref()],
            ),
            (IDMM_PACKAGE, vec![agent_session_query_service_ref()]),
            (
                REMOTE_INGRESS_PACKAGE,
                vec![
                    agent_session_command_service_ref(),
                    agent_session_query_service_ref(),
                ],
            ),
            (REQUIREMENTS_PACKAGE, Vec::new()),
        ];

        for (registration, (package_id, expected_services)) in
            registrations.iter().zip(expected)
        {
            let manifest = &registration.metadata.manifest.payload;
            assert_eq!(manifest.package_id.as_ref(), package_id);
            assert_eq!(
                manifest
                    .requires_services
                    .iter()
                    .map(|requirement| requirement.service.clone())
                    .collect::<Vec<_>>(),
                expected_services
            );
            assert!(registration
                .metadata
                .context
                .declared_services
                .required_service_handles
                .iter()
                .all(|handle| {
                    handle.provider_package == agent_core_package_ref()
                        && handle.provider_mount_id == agent_core_mount_id()
                }));
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
        assert!(descriptor.is_exact_contract());
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
        assert!(operation_from_input(
            &CapabilityId::from(REMOTE_MCP),
            StrictJsonValue(serde_json::json!({}))
        )
        .is_err());
    }

    #[test]
    fn action_mapping_is_one_to_one_and_remote_is_not_an_action() {
        let expected = [
            (
                AGENT_DELEGATE,
                AGENT_DELEGATE_ACTION,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_FORK,
                AGENT_FORK_ACTION,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_EXECUTION_PLAN,
                AGENT_EXECUTION_PLAN_ACTION,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_EXECUTION_STEER,
                AGENT_EXECUTION_STEER_ACTION,
                Wave5OwnerDomain::AgentExecution,
            ),
            (
                AGENT_EXECUTION_OBSERVE,
                AGENT_EXECUTION_OBSERVE_ACTION,
                Wave5OwnerDomain::AgentExecution,
            ),
            (SCHEDULE_STORE, SCHEDULE_STORE_ACTION, Wave5OwnerDomain::Schedule),
            (
                REQUIREMENTS_READ,
                REQUIREMENTS_READ_ACTION,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_WRITE,
                REQUIREMENTS_WRITE_ACTION,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_STATUS,
                REQUIREMENTS_STATUS_ACTION,
                Wave5OwnerDomain::Requirements,
            ),
            (
                REQUIREMENTS_CLAIM,
                REQUIREMENTS_CLAIM_ACTION,
                Wave5OwnerDomain::Requirements,
            ),
        ];

        for (capability_id, action, owner_domain) in expected {
            let operation = operation_from_input(
                &CapabilityId::from(capability_id),
                StrictJsonValue(serde_json::json!({})),
            )
            .expect("action capability must map to a typed operation");
            assert_eq!(operation.capability_id().as_ref(), capability_id);
            assert_eq!(operation.action_id().as_ref(), action);
            assert_eq!(operation.owner_domain(), owner_domain);
        }
        for capability_id in REMOTE_INGRESS_CAPABILITY_IDS {
            assert!(operation_from_input(
                &CapabilityId::from(capability_id),
                StrictJsonValue(serde_json::json!({})),
            )
            .is_err());
        }
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
            .replace_all(materializable_registrations(unconfigured_host_port()))
            .expect("Wave 5 metadata should materialize");

        assert_eq!(materialized.packages.len(), PACKAGE_IDS.len() + 1);
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
    fn handlers_are_registered_without_a_second_authority() {
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

    #[tokio::test]
    async fn unconfigured_action_host_fails_closed_without_a_synthetic_receipt() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence should initialize");
        registry
            .replace_all(materializable_registrations(unconfigured_host_port()))
            .expect("Wave 5 metadata should materialize");
        let owner = principal();
        let (snapshot, active) = compiled_schedule(&registry, &owner);
        let result = registry
            .invoke(
                &snapshot,
                &active,
                CapabilityInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner,
                    agent_session_id: AgentSessionId::from("wave5-test-session"),
                    operation_id: OperationId::from("wave5-test-operation"),
                    idempotency_key: IdempotencyKey::from("wave5-test-idempotency"),
                    correlation_id: CorrelationId::from("wave5-test-correlation"),
                    resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: CapabilityId::from(SCHEDULE_STORE),
                    action_id: ActionId::from(SCHEDULE_STORE_ACTION),
                    resource_binding_ids: BTreeSet::new(),
                    state_scope_key: nomifun_agent_contracts::ScopeKey::from(
                        "session:wave5-test",
                    ),
                    input: StrictJsonValue(serde_json::json!({})),
                },
            )
            .await
            .expect_err("unconfigured Wave 5 actions must fail closed");
        assert!(matches!(
            result,
            KernelError::CapabilityExecution { ref reason }
                if reason.starts_with("WAVE5_HOST_PORT_UNAVAILABLE:")
        ));
        assert!(!result.to_string().contains("accepted"));
    }

    #[tokio::test]
    async fn action_host_receives_the_kernel_authorized_state_handle() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence should initialize");
        registry
            .replace_all(materializable_registrations(Arc::new(StateBackedHost)))
            .expect("Wave 5 metadata should materialize");
        let owner = principal();
        let (snapshot, active) = compiled_schedule(&registry, &owner);
        let result = registry
            .invoke(
                &snapshot,
                &active,
                CapabilityInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner,
                    agent_session_id: AgentSessionId::from("wave5-state-session"),
                    operation_id: OperationId::from("wave5-state-operation"),
                    idempotency_key: IdempotencyKey::from("wave5-state-idempotency"),
                    correlation_id: CorrelationId::from("wave5-state-correlation"),
                    resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: CapabilityId::from(SCHEDULE_STORE),
                    action_id: ActionId::from(SCHEDULE_STORE_ACTION),
                    resource_binding_ids: BTreeSet::new(),
                    state_scope_key: nomifun_agent_contracts::ScopeKey::from(
                        "session:wave5-state",
                    ),
                    input: StrictJsonValue(serde_json::json!({})),
                },
            )
            .await
            .expect("state-backed action host should receive the request");
        assert_eq!(result.0["package_id"], serde_json::json!("nomifun.autowork-scheduler"));
        assert_eq!(
            result.0["mount_id"],
            serde_json::json!("nomifun-autowork-scheduler")
        );
        assert_eq!(
            result.0["state_scope"],
            serde_json::json!("session:wave5-state")
        );
        assert_eq!(result.0["state_key"], serde_json::json!("wave5-test-state"));
        assert_eq!(result.0["present"], serde_json::json!(false));
        assert_eq!(
            result.0["service_ids"],
            serde_json::json!(["service.agent-session-command.v1"])
        );
        assert_eq!(
            result.0["service_provider_mounts"],
            serde_json::json!(["platform-agent-core"])
        );
    }

    #[tokio::test]
    async fn action_host_preserves_the_owner_canonical_error_code() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence should initialize");
        registry
            .replace_all(materializable_registrations(Arc::new(
                CanonicalErrorHost,
            )))
            .expect("Wave 5 metadata should materialize");
        let owner = principal();
        let (snapshot, active) = compiled_schedule(&registry, &owner);
        let result = registry
            .invoke(
                &snapshot,
                &active,
                CapabilityInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner,
                    agent_session_id: AgentSessionId::from("wave5-error-session"),
                    operation_id: OperationId::from("wave5-error-operation"),
                    idempotency_key: IdempotencyKey::from("wave5-error-idempotency"),
                    correlation_id: CorrelationId::from("wave5-error-correlation"),
                    resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: CapabilityId::from(SCHEDULE_STORE),
                    action_id: ActionId::from(SCHEDULE_STORE_ACTION),
                    resource_binding_ids: BTreeSet::new(),
                    state_scope_key: ScopeKey::from("session:wave5-error"),
                    input: StrictJsonValue(serde_json::json!({})),
                },
            )
            .await
            .expect_err("owner error should propagate");
        assert!(matches!(
            result,
            KernelError::CapabilityExecution { ref reason }
                if reason.starts_with("REMOTE_AUTH_REQUIRED:")
        ));
    }
}
