//! Bundled Agent Capability Platform v2 registrations for C7 Wave 4.
//!
//! This crate owns only the identity/channel/device contribution metadata and
//! a typed host-port adapter.  Pairing and product-level user
//! confirmation remain transport/host concerns; they are not Agent
//! capabilities or execution branches.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityAuthoringPolicy, CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind,
    CapabilityManifest, CapabilityRef,
    CanonicalSchemaRef, CancellationDescriptor, CorrelationId, DigestHex,
    DeclaredServiceViewDescriptor, DomainOutboxPortDescriptor, EffectClass,
    HostPortBindingDescriptor, IdempotencyKey,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor, AgentModuleId,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    StrictJsonValue, ToolPresentationKind, TypedCommandPortDescriptor, TypedResourceBinding,
    TypedResourceBindings, ValidatedPluginConfig, VersionString,
    capability_module_surface_declarations, capability_surface_declarations, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityContextContributionFactory, CapabilityContextContributionRequest,
    CapabilityHandler, CapabilityInvocationContext, CapabilityResourceProviderFactory,
    CapabilityResourceProviderRequest, ContextContributionResult, KernelError,
    PluginRegistration, ResourceProviderResult,
};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const VERSION: &str = CONTRACT_VERSION;
pub const PACKAGE_VERSION: &str = CONTRACT_VERSION;

pub const CHANNEL_PACKAGE_ID: &str = "nomifun.channel";
pub const COMPANION_PACKAGE_ID: &str = "nomifun.companion";
pub const CUSTOMER_SERVICE_PACKAGE_ID: &str = "nomifun.customer-service";
pub const ROBOT_PACKAGE_ID: &str = "nomifun.robot";
pub const NOTIFICATION_PACKAGE_ID: &str = "nomifun.notification";

// Short aliases are kept for callers that use the package role names.
pub const CHANNEL_PACKAGE: &str = CHANNEL_PACKAGE_ID;
pub const COMPANION_PACKAGE: &str = COMPANION_PACKAGE_ID;
pub const CUSTOMER_SERVICE_PACKAGE: &str = CUSTOMER_SERVICE_PACKAGE_ID;
pub const ROBOT_PACKAGE: &str = ROBOT_PACKAGE_ID;
pub const NOTIFICATION_PACKAGE: &str = NOTIFICATION_PACKAGE_ID;

pub const CHANNEL_RESOURCE_KIND: &str = "channel";
pub const COMPANION_RESOURCE_KIND: &str = "companion";
pub const COMPANION_MEMORY_RESOURCE_KIND: &str = "companion_memory";
pub const CUSTOMER_RESOURCE_KIND: &str = "customer";
pub const ROBOT_RESOURCE_KIND: &str = "robot";

pub const CHANNEL_MESSAGING_MODULE_ID: &str = "channel.messaging";
pub const COMPANION_MODULE_ID: &str = "companion";
pub const COMPANION_MEMORY_MODULE_ID: &str = "companion.memory";
pub const CUSTOMER_SERVICE_MODULE_ID: &str = "customer.service";
pub const ROBOT_MODULE_ID: &str = "robot";

pub const CHANNEL_MESSAGING_REPLY_ACTION_ID: &str = "channel.messaging/reply";
pub const CHANNEL_MESSAGING_SEND_ACTION_ID: &str = "channel.messaging/send";
pub const COMPANION_LEARN_ACTION_ID: &str = "companion/learn";
pub const COMPANION_EVOLVE_ACTION_ID: &str = "companion/evolve";
pub const COMPANION_MEMORY_RECALL_ACTION_ID: &str = "companion.memory/recall";
pub const COMPANION_MEMORY_WRITE_ACTION_ID: &str = "companion.memory/write";
pub const CUSTOMER_SERVICE_NOTES_READ_ACTION_ID: &str = "customer.service/notes.read";
pub const CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID: &str = "customer.service/notes.write";
pub const CUSTOMER_SERVICE_HANDOFF_ACTION_ID: &str = "customer.service/handoff";
pub const ROBOT_VISION_ACTION_ID: &str = "robot/vision";
pub const ROBOT_DISPLAY_ACTION_ID: &str = "robot/display";
pub const ROBOT_MOTION_ACTION_ID: &str = "robot/motion";
pub const ROBOT_DEVICE_ACTION_ID: &str = "robot/device";

pub const CONVERSATION_MODULE_IDS: [&str; 3] = [
    CHANNEL_MESSAGING_MODULE_ID,
    COMPANION_MODULE_ID,
    CUSTOMER_SERVICE_MODULE_ID,
];
pub const CONVERSATION_ACTION_IDS: [&str; 7] = [
    CHANNEL_MESSAGING_REPLY_ACTION_ID,
    CHANNEL_MESSAGING_SEND_ACTION_ID,
    COMPANION_LEARN_ACTION_ID,
    COMPANION_EVOLVE_ACTION_ID,
    CUSTOMER_SERVICE_NOTES_READ_ACTION_ID,
    CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID,
    CUSTOMER_SERVICE_HANDOFF_ACTION_ID,
];

pub const CHANNEL_RECEIVE: &str = "channel.transport/receive";
pub const CHANNEL_PAIRING: &str = "channel.transport/pairing";
pub const CHANNEL_GROUP_POLICY: &str = "channel.transport/group_policy";
pub const COMPANION_PERSONA: &str = "companion.context/persona";
pub const COMPANION_ROSTER: &str = "companion.context/roster";
pub const CUSTOMER_SERVICE_DIALOGUE: &str = "customer.service/context.dialogue";
pub const PACKAGE_IDS: [&str; 5] = [
    CHANNEL_PACKAGE_ID,
    COMPANION_PACKAGE_ID,
    CUSTOMER_SERVICE_PACKAGE_ID,
    ROBOT_PACKAGE_ID,
    NOTIFICATION_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 5] = PACKAGE_IDS;

/// Exact canonical capability IDs contributed by the five target packages.
///
/// This is intentionally the full checked-in target-package inventory, not
/// only the deletion-contract family subset.
pub const TARGET_CAPABILITY_IDS: [&str; 4] = [
    CHANNEL_MESSAGING_MODULE_ID,
    COMPANION_MODULE_ID,
    CUSTOMER_SERVICE_MODULE_ID,
    ROBOT_MODULE_ID,
];
pub const CAPABILITY_IDS: [&str; 4] = TARGET_CAPABILITY_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 4] = TARGET_CAPABILITY_IDS;

const AGENT_SURFACES: &[&str] = &["desktop", "headless", "remote", "web"];
const CHANNEL_RESOURCE: &[&str] = &[CHANNEL_RESOURCE_KIND];
const COMPANION_RESOURCE: &[&str] = &[COMPANION_RESOURCE_KIND];
const CUSTOMER_RESOURCE: &[&str] = &[CUSTOMER_RESOURCE_KIND];
const ROBOT_RESOURCE: &[&str] = &[ROBOT_RESOURCE_KIND];

#[derive(Clone, Copy)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

const COMPANION_SCENE_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: COMPANION_RESOURCE_KIND,
    operation: "read",
}];
const CUSTOMER_SCENE_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CUSTOMER_RESOURCE_KIND,
    operation: "read",
}];
const CHANNEL_GROUP_POLICY_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CHANNEL_RESOURCE_KIND,
    operation: "manage",
}];

#[derive(Clone, Copy)]
struct CapabilitySpec {
    id: &'static str,
    kind: CapabilityKind,
    display_name: &'static str,
    description: &'static str,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
    effect_class: Option<EffectClass>,
}

#[derive(Clone, Copy)]
struct PortSpec {
    command_ports: &'static [&'static str],
    outbox_ports: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct PackageSpec {
    id: &'static str,
    mount_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    capabilities: &'static [CapabilitySpec],
    ports: PortSpec,
}

#[derive(Clone, Copy)]
struct ModuleActionSpec {
    id: &'static str,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
    effect_class: EffectClass,
}

#[derive(Clone, Copy)]
struct ConversationModuleSpec {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    actions: &'static [ModuleActionSpec],
    scene: Option<ConversationSceneSpec>,
}

#[derive(Clone, Copy)]
enum ConversationSceneKind {
    Context,
    TurnMiddleware,
}

#[derive(Clone, Copy)]
struct ConversationSceneSpec {
    id: &'static str,
    requirements: &'static [ResourceRequirement],
    kind: ConversationSceneKind,
}

/// Public metadata for a typed resource slot owned or consumed by this wave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedResourceDescriptor {
    pub slot_key: &'static str,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: &'static str,
}

/// The single narrow host port used by Wave 4 action handlers.
///
/// The host owns the Companion, Channel, Customer Service, and Robot facts.
/// This crate only validates the frozen invocation boundary and routes a
/// typed operation to the injected owner.  It never manufactures an action
/// result.
pub const WAVE4_CAPABILITY_HOST_PORT_ID: &str = "host.wave4.capability.invoke";
pub const WAVE4_CONTEXT_HOST_PORT_ID: &str = "host.wave4.context.contribute";
pub const WAVE4_LIFECYCLE_HOST_PORT_ID: &str = "host.wave4.lifecycle.activate";
pub const WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID: &str = "host.wave4.turn-middleware.apply";
pub const WAVE4_HOST_PORT_UNAVAILABLE: &str = "WAVE4_HOST_PORT_UNAVAILABLE";
pub const WAVE4_INVALID_REQUEST: &str = "WAVE4_INVALID_REQUEST";
pub const WAVE4_ACTION_OPERATION_MISMATCH: &str = "WAVE4_ACTION_OPERATION_MISMATCH";
pub const WAVE4_ACTION_OUTCOME_UNKNOWN: &str = "WAVE4_ACTION_OUTCOME_UNKNOWN";
pub const WAVE4_RESOURCE_BINDING_INVALID: &str = "WAVE4_RESOURCE_BINDING_INVALID";
/// Canonical admission result when a real Wave 4 owner exists but the current
/// Session/Remote/Automation target has not selected the required resource.
///
/// This is deliberately distinct from [`WAVE4_HOST_PORT_UNAVAILABLE`]: an
/// unbound Channel or Companion is a configurable target state, not evidence
/// that the bundled capability implementation is missing from this host.
pub const WAVE4_RESOURCE_NOT_BOUND: &str = nomifun_agent_contracts::PRESET_RESOURCE_NOT_BOUND;
pub const WAVE4_RESOURCE_OWNER_MISMATCH: &str = "RESOURCE_OWNER_MISMATCH";

/// Invocation metadata projected from the Kernel context into a domain port.
///
/// The projection intentionally excludes the application service bag,
/// Gateway state, and the Kernel authority itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave4HostContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub state_scope_key: ScopeKey,
    pub resource_bindings: TypedResourceBindings,
}

/// Typed operation variants understood by the Wave 4 host port.
///
/// The input remains a strict JSON value because each first-party domain owns
/// its action payload schema.  The variant itself fixes the owning domain and
/// action family before the value reaches that host adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave4CapabilityOperation {
    ChannelReply { input: StrictJsonValue },
    ChannelSend { input: StrictJsonValue },
    CompanionLearn { input: StrictJsonValue },
    CompanionEvolve { input: StrictJsonValue },
    CompanionMemoryRecall { input: StrictJsonValue },
    CompanionMemoryWrite { input: StrictJsonValue },
    CustomerServiceNotesRead { input: StrictJsonValue },
    CustomerServiceNotesWrite { input: StrictJsonValue },
    CustomerServiceHandoff { input: StrictJsonValue },
    RobotVision { input: StrictJsonValue },
    RobotDisplay { input: StrictJsonValue },
    RobotMotion { input: StrictJsonValue },
    RobotDevice { input: StrictJsonValue },
}

impl Wave4CapabilityOperation {
    /// Return the canonical capability identity fixed by this typed variant.
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::ChannelReply { .. } | Self::ChannelSend { .. } => CHANNEL_MESSAGING_MODULE_ID,
            Self::CompanionLearn { .. } | Self::CompanionEvolve { .. } => COMPANION_MODULE_ID,
            Self::CompanionMemoryRecall { .. } | Self::CompanionMemoryWrite { .. } => {
                COMPANION_MEMORY_MODULE_ID
            }
            Self::CustomerServiceNotesRead { .. }
            | Self::CustomerServiceNotesWrite { .. }
            | Self::CustomerServiceHandoff { .. } => CUSTOMER_SERVICE_MODULE_ID,
            Self::RobotVision { .. }
            | Self::RobotDisplay { .. }
            | Self::RobotMotion { .. }
            | Self::RobotDevice { .. } => ROBOT_MODULE_ID,
        })
    }

    /// Return the canonical action identity paired with this operation.
    pub fn action_id(&self) -> ActionId {
        ActionId::from(match self {
            Self::ChannelReply { .. } => CHANNEL_MESSAGING_REPLY_ACTION_ID,
            Self::ChannelSend { .. } => CHANNEL_MESSAGING_SEND_ACTION_ID,
            Self::CompanionLearn { .. } => COMPANION_LEARN_ACTION_ID,
            Self::CompanionEvolve { .. } => COMPANION_EVOLVE_ACTION_ID,
            Self::CompanionMemoryRecall { .. } => COMPANION_MEMORY_RECALL_ACTION_ID,
            Self::CompanionMemoryWrite { .. } => COMPANION_MEMORY_WRITE_ACTION_ID,
            Self::CustomerServiceNotesRead { .. } => CUSTOMER_SERVICE_NOTES_READ_ACTION_ID,
            Self::CustomerServiceNotesWrite { .. } => CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID,
            Self::CustomerServiceHandoff { .. } => CUSTOMER_SERVICE_HANDOFF_ACTION_ID,
            Self::RobotVision { .. } => ROBOT_VISION_ACTION_ID,
            Self::RobotDisplay { .. } => ROBOT_DISPLAY_ACTION_ID,
            Self::RobotMotion { .. } => ROBOT_MOTION_ACTION_ID,
            Self::RobotDevice { .. } => ROBOT_DEVICE_ACTION_ID,
        })
    }

    /// Return the first-party owner domain for the operation.
    pub fn owner_domain(&self) -> Wave4OwnerDomain {
        match self {
            Self::ChannelReply { .. } | Self::ChannelSend { .. } => Wave4OwnerDomain::Channel,
            Self::CompanionLearn { .. }
            | Self::CompanionEvolve { .. }
            | Self::CompanionMemoryRecall { .. }
            | Self::CompanionMemoryWrite { .. } => Wave4OwnerDomain::Companion,
            Self::CustomerServiceNotesRead { .. }
            | Self::CustomerServiceNotesWrite { .. }
            | Self::CustomerServiceHandoff { .. } => Wave4OwnerDomain::CustomerService,
            Self::RobotVision { .. }
            | Self::RobotDisplay { .. }
            | Self::RobotMotion { .. }
            | Self::RobotDevice { .. } => Wave4OwnerDomain::Robot,
        }
    }

    fn input(&self) -> &StrictJsonValue {
        match self {
            Self::ChannelReply { input }
            | Self::ChannelSend { input }
            | Self::CompanionLearn { input }
            | Self::CompanionEvolve { input }
            | Self::CompanionMemoryRecall { input }
            | Self::CompanionMemoryWrite { input }
            | Self::CustomerServiceNotesRead { input }
            | Self::CustomerServiceNotesWrite { input }
            | Self::CustomerServiceHandoff { input }
            | Self::RobotVision { input }
            | Self::RobotDisplay { input }
            | Self::RobotMotion { input }
            | Self::RobotDevice { input } => input,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave4HostRequest {
    pub context: Wave4HostContext,
    pub operation: Wave4CapabilityOperation,
}

impl Wave4HostRequest {
    /// Validate the complete boundary before an owner receives the request.
    ///
    /// This is intentionally public so central composition and future owners
    /// can apply the same fail-closed contract when they receive a request
    /// outside the Kernel handler path.
    pub fn validate(&self) -> Result<(), Wave4HostPortError> {
        let capability_id = &self.context.capability_id;
        let Some(requirements) = action_requirements(
            capability_id.as_ref(),
            self.context.action_id.as_ref(),
        ) else {
            return Err(Wave4HostPortError::invalid_request(format!(
                "unknown Wave 4 Module/Action {}/{}",
                capability_id.as_ref(),
                self.context.action_id.as_ref()
            )));
        };

        let operation_capability_id = self.operation.capability_id();
        let operation_action_id = self.operation.action_id();
        if operation_capability_id != *capability_id
            || operation_action_id != self.context.action_id
        {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "context maps {} / {} but typed operation maps {} / {}",
                capability_id.as_ref(),
                self.context.action_id.as_ref(),
                operation_capability_id.as_ref(),
                operation_action_id.as_ref()
            )));
        }
        if !self.operation.input().0.is_object() {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{} input must be a JSON object",
                capability_id.as_ref()
            )));
        }

        validate_host_context(&self.context)?;
        validate_resource_bindings_contract(
            capability_id,
            &self.context.principal.principal_id,
            requirements,
            &self.context.resource_bindings,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave4HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(WAVE4_HOST_PORT_UNAVAILABLE, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(WAVE4_INVALID_REQUEST, message)
    }

    pub fn action_operation_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE4_ACTION_OPERATION_MISMATCH, message)
    }

    pub fn resource_binding_invalid(message: impl Into<String>) -> Self {
        Self::new(WAVE4_RESOURCE_BINDING_INVALID, message)
    }

    pub fn resource_not_bound(message: impl Into<String>) -> Self {
        Self::new(WAVE4_RESOURCE_NOT_BOUND, message)
    }

    pub fn resource_owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE4_RESOURCE_OWNER_MISMATCH, message)
    }
}

impl fmt::Display for Wave4HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave4HostPortError {}

/// Production-owned implementation boundary for Wave 4 actions.
pub trait Wave4HostPort: Send + Sync {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>>;
}

/// Invocation metadata for a host-owned Wave 4 Context contribution.
///
/// Context contributions are deliberately separate from action invocation:
/// they have no synthetic action/idempotency identity and cannot be exposed as
/// a model Tool just to make a catalog entry appear usable.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave4ContextHostRequest {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub registry_digest: DigestHex,
    pub capability_id: CapabilityId,
    pub state_scope_key: ScopeKey,
    pub resource_bindings: TypedResourceBindings,
    pub schema_ref: CanonicalSchemaRef,
}

impl Wave4ContextHostRequest {
    pub fn validate(&self) -> Result<(), Wave4HostPortError> {
        let requirements = match find_capability(self.capability_id.as_ref()) {
            Some(spec) if spec.kind == CapabilityKind::ContextContributor => spec.requirements,
            Some(_) => {
                return Err(Wave4HostPortError::action_operation_mismatch(format!(
                    "{} is not a Context contribution",
                    self.capability_id.as_ref()
                )));
            }
            None
                if matches!(
                    self.capability_id.as_ref(),
                    COMPANION_PERSONA | COMPANION_ROSTER
                ) => COMPANION_SCENE_READ_REQUIREMENTS,
            None => {
                return Err(Wave4HostPortError::invalid_request(format!(
                    "unknown Wave 4 Context capability {}",
                    self.capability_id.as_ref()
                )));
            }
        };
        let fields = [
            ("principal.principal_kind", self.principal.principal_kind.as_str()),
            ("principal.principal_id", self.principal.principal_id.as_str()),
            ("agent_session_id", self.agent_session_id.as_ref()),
            ("operation_id", self.operation_id.as_ref()),
            ("correlation_id", self.correlation_id.as_ref()),
            (
                "resolved_snapshot_ref.snapshot_id",
                self.resolved_snapshot_ref.snapshot_id.as_ref(),
            ),
            (
                "resolved_snapshot_ref.snapshot_digest",
                self.resolved_snapshot_ref.snapshot_digest.as_ref(),
            ),
            ("registry_digest", self.registry_digest.as_ref()),
            ("state_scope_key", self.state_scope_key.as_ref()),
            ("schema_ref", self.schema_ref.as_ref()),
        ];
        if let Some((field, _)) = fields
            .iter()
            .find(|(_, value)| value.trim().is_empty())
        {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{field} must be non-empty"
            )));
        }
        if self.registry_generation == 0 {
            return Err(Wave4HostPortError::invalid_request(
                "registry_generation must identify a published generation",
            ));
        }
        let expected_schema = schema_ref(
            self.capability_id.as_ref(),
            "context",
            &context_output_schema(self.capability_id.as_ref()),
        )
        .map_err(Wave4HostPortError::invalid_request)?;
        if self.schema_ref != expected_schema {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{} received a non-canonical Context schema",
                self.capability_id.as_ref()
            )));
        }
        validate_resource_bindings_contract(
            &self.capability_id,
            &self.principal.principal_id,
            requirements,
            &self.resource_bindings,
        )
    }
}

/// Production owner for Wave 4 Context contributions.
pub trait Wave4ContextHostPort: Send + Sync {
    fn contribute<'a>(
        &'a self,
        request: Wave4ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<StrictJsonValue>, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    >;
}

struct UnconfiguredWave4HostPort;

impl Wave4HostPort for UnconfiguredWave4HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            Err(Wave4HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

/// Return the fail-closed adapter used by metadata-only compositions.
pub fn unconfigured_host_port() -> Arc<dyn Wave4HostPort> {
    Arc::new(UnconfiguredWave4HostPort)
}

struct UnconfiguredWave4ContextHostPort;

impl Wave4ContextHostPort for UnconfiguredWave4ContextHostPort {
    fn contribute<'a>(
        &'a self,
        request: Wave4ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<StrictJsonValue>, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            request.validate()?;
            Err(Wave4HostPortError::unavailable(format!(
                "no production Context owner is bound for {}",
                request.capability_id.as_ref()
            )))
        })
    }
}

pub fn unconfigured_context_host_port() -> Arc<dyn Wave4ContextHostPort> {
    Arc::new(UnconfiguredWave4ContextHostPort)
}

/// One exact per-turn middleware request. Unlike Context contributions, this
/// boundary may inspect the current turn input and apply domain policy before
/// the model is invoked; it therefore has its own host port and cannot be
/// projected as a synthetic ContextContributor.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave4TurnMiddlewareHostRequest {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub registry_digest: DigestHex,
    pub capability_id: CapabilityId,
    pub state_scope_key: ScopeKey,
    pub resource_bindings: TypedResourceBindings,
    pub schema_ref: CanonicalSchemaRef,
    pub turn_input: StrictJsonValue,
}

impl Wave4TurnMiddlewareHostRequest {
    pub fn validate(&self) -> Result<(), Wave4HostPortError> {
        let requirements = match find_capability(self.capability_id.as_ref()) {
            Some(spec) if spec.kind == CapabilityKind::TurnMiddleware => spec.requirements,
            Some(_) => {
                return Err(Wave4HostPortError::action_operation_mismatch(format!(
                    "{} is not a TurnMiddleware capability",
                    self.capability_id.as_ref()
                )));
            }
            None if self.capability_id.as_ref() == CHANNEL_GROUP_POLICY => {
                CHANNEL_GROUP_POLICY_REQUIREMENTS
            }
            None if self.capability_id.as_ref() == CUSTOMER_SERVICE_DIALOGUE => {
                CUSTOMER_SCENE_READ_REQUIREMENTS
            }
            None => {
                return Err(Wave4HostPortError::invalid_request(format!(
                    "unknown Wave 4 TurnMiddleware capability {}",
                    self.capability_id.as_ref()
                )));
            }
        };
        if !self.turn_input.0.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Wave 4 TurnMiddleware input must be a JSON object",
            ));
        }
        let fields = [
            ("principal.principal_kind", self.principal.principal_kind.as_str()),
            ("principal.principal_id", self.principal.principal_id.as_str()),
            ("agent_session_id", self.agent_session_id.as_ref()),
            ("operation_id", self.operation_id.as_ref()),
            ("correlation_id", self.correlation_id.as_ref()),
            (
                "resolved_snapshot_ref.snapshot_id",
                self.resolved_snapshot_ref.snapshot_id.as_ref(),
            ),
            (
                "resolved_snapshot_ref.snapshot_digest",
                self.resolved_snapshot_ref.snapshot_digest.as_ref(),
            ),
            ("registry_digest", self.registry_digest.as_ref()),
            ("state_scope_key", self.state_scope_key.as_ref()),
            ("schema_ref", self.schema_ref.as_ref()),
        ];
        if let Some((field, _)) = fields
            .iter()
            .find(|(_, value)| value.trim().is_empty())
        {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{field} must be non-empty"
            )));
        }
        if self.registry_generation == 0 {
            return Err(Wave4HostPortError::invalid_request(
                "registry_generation must identify a published generation",
            ));
        }
        let expected_schema = schema_ref(
            self.capability_id.as_ref(),
            "context",
            &context_output_schema(self.capability_id.as_ref()),
        )
        .map_err(Wave4HostPortError::invalid_request)?;
        if self.schema_ref != expected_schema {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{} received a non-canonical TurnMiddleware schema",
                self.capability_id.as_ref()
            )));
        }
        validate_resource_bindings_contract(
            &self.capability_id,
            &self.principal.principal_id,
            requirements,
            &self.resource_bindings,
        )
    }
}

/// Product owner for one Wave 4 per-turn middleware contribution.
pub trait Wave4TurnMiddlewareHostPort: Send + Sync {
    fn apply<'a>(
        &'a self,
        request: Wave4TurnMiddlewareHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>>;
}

struct UnconfiguredWave4TurnMiddlewareHostPort;

impl Wave4TurnMiddlewareHostPort for UnconfiguredWave4TurnMiddlewareHostPort {
    fn apply<'a>(
        &'a self,
        request: Wave4TurnMiddlewareHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            Err(Wave4HostPortError::unavailable(format!(
                "no production TurnMiddleware owner is bound for {}",
                request.capability_id.as_ref()
            )))
        })
    }
}

pub fn unconfigured_turn_middleware_host_port() -> Arc<dyn Wave4TurnMiddlewareHostPort> {
    Arc::new(UnconfiguredWave4TurnMiddlewareHostPort)
}

/// The owner domains that may be injected independently by central
/// composition.  Pairing/transport remains outside this enum by design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave4OwnerDomain {
    Channel,
    Companion,
    CustomerService,
    Robot,
}

/// Optional first-party owner bindings for the canonical Wave 4 action port.
///
/// Each owner implements the same typed port, while central composition can
/// provide only the owners that are real.  Missing owners remain unavailable;
/// this type never supplies a success fallback.
#[derive(Default)]
pub struct Wave4OwnerBindings {
    pub channel: Option<Arc<dyn Wave4HostPort>>,
    pub companion: Option<Arc<dyn Wave4HostPort>>,
    pub customer_service: Option<Arc<dyn Wave4HostPort>>,
    pub robot: Option<Arc<dyn Wave4HostPort>>,
}

impl Wave4OwnerBindings {
    pub fn with_channel(mut self, owner: Arc<dyn Wave4HostPort>) -> Self {
        self.channel = Some(owner);
        self
    }

    pub fn with_companion(mut self, owner: Arc<dyn Wave4HostPort>) -> Self {
        self.companion = Some(owner);
        self
    }

    pub fn with_customer_service(mut self, owner: Arc<dyn Wave4HostPort>) -> Self {
        self.customer_service = Some(owner);
        self
    }

    pub fn with_robot(mut self, owner: Arc<dyn Wave4HostPort>) -> Self {
        self.robot = Some(owner);
        self
    }
}

/// Compose independently injected owners behind the one manifest host port.
pub fn composed_host_port(bindings: Wave4OwnerBindings) -> Arc<dyn Wave4HostPort> {
    Arc::new(ComposedWave4HostPort { bindings })
}

struct ComposedWave4HostPort {
    bindings: Wave4OwnerBindings,
}

impl Wave4HostPort for ComposedWave4HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>>
    {
        if let Err(error) = request.validate() {
            return Box::pin(async move { Err(error) });
        }

        let owner = match request.operation.owner_domain() {
            Wave4OwnerDomain::Channel => self.bindings.channel.clone(),
            Wave4OwnerDomain::Companion => self.bindings.companion.clone(),
            Wave4OwnerDomain::CustomerService => self.bindings.customer_service.clone(),
            Wave4OwnerDomain::Robot => self.bindings.robot.clone(),
        };
        let capability_id = request.context.capability_id.clone();
        Box::pin(async move {
            let Some(owner) = owner else {
                return Err(Wave4HostPortError::unavailable(format!(
                    "no production owner is bound for {}",
                    capability_id.as_ref()
                )));
            };
            owner.invoke(request).await
        })
    }
}

/// Independently mounted owners for Wave 4 Context contributions.
#[derive(Default)]
pub struct Wave4ContextOwnerBindings {
    pub companion: Option<Arc<dyn Wave4ContextHostPort>>,
}

impl Wave4ContextOwnerBindings {
    pub fn with_companion(mut self, owner: Arc<dyn Wave4ContextHostPort>) -> Self {
        self.companion = Some(owner);
        self
    }

}

pub fn composed_context_host_port(
    bindings: Wave4ContextOwnerBindings,
) -> Arc<dyn Wave4ContextHostPort> {
    Arc::new(ComposedWave4ContextHostPort { bindings })
}

struct ComposedWave4ContextHostPort {
    bindings: Wave4ContextOwnerBindings,
}

impl Wave4ContextHostPort for ComposedWave4ContextHostPort {
    fn contribute<'a>(
        &'a self,
        request: Wave4ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<StrictJsonValue>, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        if let Err(error) = request.validate() {
            return Box::pin(async move { Err(error) });
        }
        let owner = match request.capability_id.as_ref() {
            COMPANION_PERSONA | COMPANION_ROSTER => self.bindings.companion.clone(),
            _ => None,
        };
        let capability_id = request.capability_id.clone();
        Box::pin(async move {
            let Some(owner) = owner else {
                return Err(Wave4HostPortError::unavailable(format!(
                    "no production Context owner is bound for {}",
                    capability_id.as_ref()
                )));
            };
            owner.contribute(request).await
        })
    }
}

const CHANNEL_MESSAGING_ACTIONS: [ModuleActionSpec; 2] = [
    ModuleActionSpec {
        id: CHANNEL_MESSAGING_REPLY_ACTION_ID,
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "reply",
        }],
        effect_class: EffectClass::ExternalTransmit,
    },
    ModuleActionSpec {
        id: CHANNEL_MESSAGING_SEND_ACTION_ID,
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "send",
        }],
        effect_class: EffectClass::ExternalTransmit,
    },
];

const COMPANION_ACTIONS: [ModuleActionSpec; 2] = [
    ModuleActionSpec {
        id: COMPANION_LEARN_ACTION_ID,
        resource_kinds: COMPANION_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: EffectClass::WriteDurable,
    },
    ModuleActionSpec {
        id: COMPANION_EVOLVE_ACTION_ID,
        resource_kinds: COMPANION_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: EffectClass::WriteDurable,
    },
];

const CUSTOMER_SERVICE_ACTIONS: [ModuleActionSpec; 3] = [
    ModuleActionSpec {
        id: CUSTOMER_SERVICE_NOTES_READ_ACTION_ID,
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: EffectClass::ReadSensitive,
    },
    ModuleActionSpec {
        id: CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID,
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: EffectClass::WriteDurable,
    },
    ModuleActionSpec {
        id: CUSTOMER_SERVICE_HANDOFF_ACTION_ID,
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: EffectClass::ExternalTransmit,
    },
];

const CHANNEL_MODULES: [ConversationModuleSpec; 1] = [ConversationModuleSpec {
    id: CHANNEL_MESSAGING_MODULE_ID,
    display_name: "Channel Messaging",
    description: "Reply and send through the selected Channel scene.",
    actions: &CHANNEL_MESSAGING_ACTIONS,
    scene: Some(ConversationSceneSpec {
        id: CHANNEL_GROUP_POLICY,
        requirements: CHANNEL_GROUP_POLICY_REQUIREMENTS,
        kind: ConversationSceneKind::TurnMiddleware,
    }),
}];
const COMPANION_MODULES: [ConversationModuleSpec; 1] = [
    ConversationModuleSpec {
        id: COMPANION_MODULE_ID,
        display_name: "Companion",
        description: "Learn and evolve the selected Companion.",
        actions: &COMPANION_ACTIONS,
        scene: Some(ConversationSceneSpec {
            id: COMPANION_PERSONA,
            requirements: COMPANION_SCENE_READ_REQUIREMENTS,
            kind: ConversationSceneKind::Context,
        }),
    },
];
const CUSTOMER_SERVICE_MODULES: [ConversationModuleSpec; 1] = [ConversationModuleSpec {
    id: CUSTOMER_SERVICE_MODULE_ID,
    display_name: "Customer Service",
    description: "Read notes, write notes, and hand off the selected customer conversation.",
    actions: &CUSTOMER_SERVICE_ACTIONS,
    scene: Some(ConversationSceneSpec {
        id: CUSTOMER_SERVICE_DIALOGUE,
        requirements: CUSTOMER_SCENE_READ_REQUIREMENTS,
        kind: ConversationSceneKind::TurnMiddleware,
    }),
}];

const ROBOT_ACTIONS: [ModuleActionSpec; 4] = [
    ModuleActionSpec {
        id: ROBOT_VISION_ACTION_ID,
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "vision",
        }],
        effect_class: EffectClass::ReadSensitive,
    },
    ModuleActionSpec {
        id: ROBOT_DISPLAY_ACTION_ID,
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "display",
        }],
        effect_class: EffectClass::Physical,
    },
    ModuleActionSpec {
        id: ROBOT_MOTION_ACTION_ID,
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "motion",
        }],
        effect_class: EffectClass::Physical,
    },
    ModuleActionSpec {
        id: ROBOT_DEVICE_ACTION_ID,
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "device",
        }],
        effect_class: EffectClass::Physical,
    },
];

const ROBOT_MODULES: [ConversationModuleSpec; 1] = [ConversationModuleSpec {
    id: ROBOT_MODULE_ID,
    display_name: "Robot",
    description: "Observe and control one explicitly bound Robot device.",
    actions: &ROBOT_ACTIONS,
    scene: None,
}];

const CHANNEL_PORTS: PortSpec = PortSpec {
    command_ports: &["channel.agent-session-command", "channel.inbound-receipt"],
    outbox_ports: &[],
};
const COMPANION_PORTS: PortSpec = PortSpec {
    command_ports: &["companion.agent-session-command"],
    outbox_ports: &[],
};
const CUSTOMER_SERVICE_PORTS: PortSpec = PortSpec {
    command_ports: &[
        "customer-service.dialogue-command",
        "customer-service.handoff-command",
    ],
    outbox_ports: &[],
};
const ROBOT_PORTS: PortSpec = PortSpec {
    command_ports: &["robot.agent-session-command", "robot.effect-command"],
    outbox_ports: &[],
};
const NOTIFICATION_PORTS: PortSpec = PortSpec {
    command_ports: &[],
    outbox_ports: &["notification.webhook-outbox"],
};

const PACKAGE_SPECS: [PackageSpec; 5] = [
    PackageSpec {
        id: CHANNEL_PACKAGE_ID,
        mount_id: "domain-channel",
        display_name: "Channel",
        description: "Bundled channel ingress and delivery capabilities.",
        capabilities: &[],
        ports: CHANNEL_PORTS,
    },
    PackageSpec {
        id: COMPANION_PACKAGE_ID,
        mount_id: "domain-companion",
        display_name: "Companion",
        description: "Bundled Companion persona, learning, and evolution capabilities.",
        capabilities: &[],
        ports: COMPANION_PORTS,
    },
    PackageSpec {
        id: CUSTOMER_SERVICE_PACKAGE_ID,
        mount_id: "domain-customer-service",
        display_name: "Customer Service",
        description: "Bundled customer dialogue and handoff capabilities.",
        capabilities: &[],
        ports: CUSTOMER_SERVICE_PORTS,
    },
    PackageSpec {
        id: ROBOT_PACKAGE_ID,
        mount_id: "domain-robot",
        display_name: "Robot",
        description: "Bundled Robot media, display, motion, and device capabilities.",
        capabilities: &[],
        ports: ROBOT_PORTS,
    },
    PackageSpec {
        id: NOTIFICATION_PACKAGE_ID,
        mount_id: "domain-notification",
        display_name: "Notification",
        description: "Bundled webhook event consumption.",
        capabilities: &[],
        ports: NOTIFICATION_PORTS,
    },
];

/// Return the exact target IDs as contract newtypes.
pub fn target_capability_ids() -> BTreeSet<CapabilityId> {
    TARGET_CAPABILITY_IDS
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

/// Return resource kinds and operations needed by the Wave 4 capabilities.
pub fn resource_binding_metadata() -> BTreeMap<ResourceKind, BTreeSet<String>> {
    typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect()
}

/// Return the typed resource slots used by the identity/channel/device slice.
pub fn typed_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    vec![
        descriptor(
            "channel",
            CHANNEL_RESOURCE_KIND,
            true,
            ["manage", "receive", "reply", "send"],
            "require_explicit_selection",
        ),
        descriptor(
            "companion",
            COMPANION_RESOURCE_KIND,
            true,
            ["read", "write"],
            "require_explicit_selection",
        ),
        descriptor(
            "companion_memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            false,
            ["read", "write"],
            "select_only_owned_resource",
        ),
        descriptor(
            "customer",
            CUSTOMER_RESOURCE_KIND,
            true,
            ["read", "write"],
            "require_explicit_selection",
        ),
        descriptor(
            "robot",
            ROBOT_RESOURCE_KIND,
            true,
            ["device", "display", "motion", "vision"],
            "require_explicit_selection",
        ),
    ]
}

/// Alias for generic callers that use the shorter resource terminology.
pub fn all_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

/// Alias for generic callers that use the shorter resource terminology.
pub fn resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

/// Build deterministic, owner-scoped bindings for the five Wave 4 slots.
///
/// These are contract fixtures, not product-resource creation.  A caller may
/// replace the concrete resource IDs before saving an AgentPreset revision.
pub fn canonical_resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    let owner_id = owner_id.into();
    vec![
        typed_resource_binding(
            "wave4-channel",
            CHANNEL_RESOURCE_KIND,
            "channel",
            &owner_id,
            ["manage", "receive", "reply", "send"],
        ),
        typed_resource_binding(
            "wave4-companion",
            COMPANION_RESOURCE_KIND,
            "companion",
            &owner_id,
            ["read", "write"],
        ),
        typed_resource_binding(
            "wave4-companion-memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            "companion-memory",
            &owner_id,
            ["read", "write"],
        ),
        typed_resource_binding(
            "wave4-customer",
            CUSTOMER_RESOURCE_KIND,
            "customer",
            &owner_id,
            ["read", "write"],
        ),
        typed_resource_binding(
            "wave4-robot",
            ROBOT_RESOURCE_KIND,
            "robot",
            &owner_id,
            ["device", "display", "motion", "vision"],
        ),
    ]
}

/// Alias for callers that already use the contract's binding terminology.
pub fn resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    canonical_resource_bindings(owner_id)
}

/// Construct one typed binding without resolving or creating a product
/// resource.
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

/// Return the resource kinds required by one target capability.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    if let Some(module) = [
        CHANNEL_MODULES.as_slice(),
        COMPANION_MODULES.as_slice(),
        CUSTOMER_SERVICE_MODULES.as_slice(),
        ROBOT_MODULES.as_slice(),
    ]
    .into_iter()
    .flatten()
    .find(|module| module.id == capability_id)
    {
        return Some(
            module
                .actions
                .iter()
                .flat_map(|action| action.resource_kinds.iter())
                .map(|kind| ResourceKind::from(*kind))
                .collect(),
        );
    }
    find_capability(capability_id).map(|spec| {
        spec.resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
}

pub fn required_action_resource_operations(
    capability_id: &str,
    action_id: &str,
) -> Option<Vec<(ResourceKind, String)>> {
    Some(
        action_requirements(capability_id, action_id)?
            .iter()
            .map(|requirement| {
                (
                    ResourceKind::from(requirement.resource_kind),
                    requirement.operation.to_owned(),
                )
            })
            .collect(),
    )
}

/// Resolve the only action identity that may be used for a capability.
pub fn canonical_action_id(capability_id: &str) -> Option<ActionId> {
    if [
        CHANNEL_MODULES.as_slice(),
        COMPANION_MODULES.as_slice(),
        CUSTOMER_SERVICE_MODULES.as_slice(),
        ROBOT_MODULES.as_slice(),
    ]
    .into_iter()
    .flatten()
    .any(|module| module.id == capability_id)
    {
        return None;
    }
    find_capability(capability_id)
        .filter(|spec| spec.effect_class.is_some())
        .map(|_| action_id_for(capability_id))
}

/// Resolve schema bytes owned by a Wave 4 capability contribution.
///
/// Nomi-core uses this source when it materializes an explicitly admitted
/// bundled Tool/Context contribution. Returning the exact bytes here avoids a
/// second permissive schema table in application composition.
pub fn resolve_capability_schema(
    reference: &CanonicalSchemaRef,
) -> Result<Option<StrictJsonValue>, String> {
    for module in [
        CHANNEL_MODULES.as_slice(),
        COMPANION_MODULES.as_slice(),
        CUSTOMER_SERVICE_MODULES.as_slice(),
        ROBOT_MODULES.as_slice(),
    ]
    .into_iter()
    .flatten()
    {
        for action in module.actions {
            let input_schema = action_input_schema(action.id);
            if schema_ref(action.id, "input", &input_schema)? == *reference {
                return Ok(Some(input_schema));
            }
            let output_schema = object_schema(true);
            if schema_ref(action.id, "output", &output_schema)? == *reference {
                return Ok(Some(output_schema));
            }
        }
    }
    for capability_id in [
        CHANNEL_GROUP_POLICY,
        COMPANION_PERSONA,
        COMPANION_ROSTER,
        CUSTOMER_SERVICE_DIALOGUE,
    ] {
        let context_schema = context_output_schema(capability_id);
        if schema_ref(capability_id, "context", &context_schema)? == *reference {
            return Ok(Some(context_schema));
        }
    }
    for capability in all_capabilities() {
        if capability.effect_class.is_some() {
            let input_schema = action_input_schema(capability.id);
            if schema_ref(capability.id, "input", &input_schema)? == *reference {
                return Ok(Some(input_schema));
            }
            let output_schema = object_schema(true);
            if schema_ref(capability.id, "output", &output_schema)? == *reference {
                return Ok(Some(output_schema));
            }
        }
        if matches!(
            capability.kind,
            CapabilityKind::ContextContributor | CapabilityKind::TurnMiddleware
        ) {
            let context_schema = context_output_schema(capability.id);
            if schema_ref(capability.id, "context", &context_schema)? == *reference {
                return Ok(Some(context_schema));
            }
        }
        if matches!(
            capability.kind,
            CapabilityKind::EventSource | CapabilityKind::EventConsumer
        ) {
            let event_schema = object_schema(true);
            if schema_ref(capability.id, "event", &event_schema)? == *reference {
                return Ok(Some(event_schema));
            }
        }
    }
    Ok(None)
}

/// Resolve a canonical schema reference for product-scene Context that is
/// derived from a selected resource binding rather than granted as an Agent
/// Capability. These identities must never appear in the authoring catalog.
pub fn scene_context_schema_ref(
    scene_id: &str,
) -> Result<Option<CanonicalSchemaRef>, String> {
    if !matches!(
        scene_id,
        CHANNEL_GROUP_POLICY
            | COMPANION_PERSONA
            | COMPANION_ROSTER
            | CUSTOMER_SERVICE_DIALOGUE
    ) {
        return Ok(None);
    }
    Ok(Some(schema_ref(
        scene_id,
        "context",
        &context_output_schema(scene_id),
    )?))
}

/// Construct all five bundled Wave 4 registrations.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Construct all five bundled Wave 4 registrations with the host-owned
/// action port.
pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave4HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_ports(action_host_port, unconfigured_context_host_port())
}

pub fn registrations_with_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    registrations_with_all_host_ports(
        action_host_port,
        context_host_port,
        unconfigured_turn_middleware_host_port(),
    )
}

pub fn registrations_with_all_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
    turn_middleware_host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    PACKAGE_SPECS
        .iter()
        .map(|spec| {
            registration_for(
                spec,
                Arc::clone(&action_host_port),
                Arc::clone(&context_host_port),
                Arc::clone(&turn_middleware_host_port),
            )
        })
        .collect()
}

pub fn channel_registration() -> Result<PluginRegistration, String> {
    channel_registration_with_host_ports(
        unconfigured_host_port(),
        unconfigured_context_host_port(),
    )
}

pub fn channel_registration_with_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<PluginRegistration, String> {
    channel_registration_with_all_host_ports(
        action_host_port,
        context_host_port,
        unconfigured_turn_middleware_host_port(),
    )
}

pub fn channel_registration_with_all_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
    turn_middleware_host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
) -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[0],
        action_host_port,
        context_host_port,
        turn_middleware_host_port,
    )
}

pub fn companion_registration() -> Result<PluginRegistration, String> {
    companion_registration_with_host_ports(
        unconfigured_host_port(),
        unconfigured_context_host_port(),
    )
}

pub fn companion_registration_with_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[1],
        action_host_port,
        context_host_port,
        unconfigured_turn_middleware_host_port(),
    )
}

pub fn customer_service_registration() -> Result<PluginRegistration, String> {
    customer_service_registration_with_host_port(unconfigured_host_port())
}

pub fn customer_service_registration_with_host_port(
    action_host_port: Arc<dyn Wave4HostPort>,
) -> Result<PluginRegistration, String> {
    customer_service_registration_with_host_ports(
        action_host_port,
        unconfigured_context_host_port(),
    )
}

pub fn customer_service_registration_with_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<PluginRegistration, String> {
    customer_service_registration_with_all_host_ports(
        action_host_port,
        context_host_port,
        unconfigured_turn_middleware_host_port(),
    )
}

pub fn customer_service_registration_with_all_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
    turn_middleware_host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
) -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[2],
        action_host_port,
        context_host_port,
        turn_middleware_host_port,
    )
}

pub fn robot_registration() -> Result<PluginRegistration, String> {
    robot_registration_with_host_ports(
        unconfigured_host_port(),
        unconfigured_context_host_port(),
    )
}

pub fn robot_registration_with_host_ports(
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[3],
        action_host_port,
        context_host_port,
        unconfigured_turn_middleware_host_port(),
    )
}

pub fn notification_registration() -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[4],
        unconfigured_host_port(),
        unconfigured_context_host_port(),
        unconfigured_turn_middleware_host_port(),
    )
}

fn all_capabilities() -> impl Iterator<Item = &'static CapabilitySpec> {
    PACKAGE_SPECS
        .iter()
        .flat_map(|package| package.capabilities.iter())
}

fn find_capability(capability_id: &str) -> Option<&'static CapabilitySpec> {
    all_capabilities().find(|spec| spec.id == capability_id)
}

fn conversation_modules_for_package(package_id: &str) -> Option<&'static [ConversationModuleSpec]> {
    match package_id {
        CHANNEL_PACKAGE_ID => Some(&CHANNEL_MODULES),
        COMPANION_PACKAGE_ID => Some(&COMPANION_MODULES),
        CUSTOMER_SERVICE_PACKAGE_ID => Some(&CUSTOMER_SERVICE_MODULES),
        ROBOT_PACKAGE_ID => Some(&ROBOT_MODULES),
        _ => None,
    }
}

fn find_conversation_action(
    capability_id: &str,
    action_id: &str,
) -> Option<&'static ModuleActionSpec> {
    [
        CHANNEL_MODULES.as_slice(),
        COMPANION_MODULES.as_slice(),
        CUSTOMER_SERVICE_MODULES.as_slice(),
        ROBOT_MODULES.as_slice(),
    ]
    .into_iter()
    .flatten()
    .find(|module| module.id == capability_id)?
    .actions
    .iter()
    .find(|action| action.id == action_id)
}

fn action_requirements(
    capability_id: &str,
    action_id: &str,
) -> Option<&'static [ResourceRequirement]> {
    if let Some(action) = find_conversation_action(capability_id, action_id) {
        return Some(action.requirements);
    }
    let spec = find_capability(capability_id)?;
    (spec.effect_class.is_some() && action_id_for(capability_id).as_ref() == action_id)
        .then_some(spec.requirements)
}

fn registration_for(
    spec: &PackageSpec,
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
    turn_middleware_host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
) -> Result<PluginRegistration, String> {
    if let Some(modules) = conversation_modules_for_package(spec.id) {
        return conversation_module_registration(
            spec,
            modules,
            action_host_port,
            context_host_port,
            turn_middleware_host_port,
        );
    }
    let package = package_ref(spec.id);
    let config_schema = object_schema(false);
    let capabilities = spec
        .capabilities
        .iter()
        .copied()
        .map(|capability| capability_manifest(&package, capability))
        .collect::<Result<Vec<_>, _>>()?;

    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: localized(spec.display_name, spec.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", spec.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        }
        .into(),
        contributions: PackageContributions {
            capabilities,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
        },
    };

    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: spec.id.to_owned(),
        source_digest: None,
    };
    let mount_id = AgentModuleId::from(spec.mount_id);
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount_id.clone(),
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let has_action_handler = spec
        .capabilities
        .iter()
        .any(|capability| capability.effect_class.is_some());
    let has_context_factory = spec
        .capabilities
        .iter()
        .any(|capability| capability.kind == CapabilityKind::ContextContributor);
    let has_turn_middleware = spec
        .capabilities
        .iter()
        .any(|capability| capability.kind == CapabilityKind::TurnMiddleware);
    let has_lifecycle = spec.capabilities.iter().any(|capability| {
        matches!(
            capability.kind,
            CapabilityKind::EventSource | CapabilityKind::Transport
        )
    });
    let typed_command_ports = spec
        .ports
        .command_ports
        .iter()
        .map(|id| command_port(id))
        .collect::<Result<Vec<_>, _>>()?;
    let domain_outbox_ports = spec
        .ports
        .outbox_ports
        .iter()
        .map(|id| outbox_port(id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut declared_host_ports = typed_command_ports
        .iter()
        .map(|port| port.port.id.clone())
        .chain(domain_outbox_ports.iter().map(|port| port.port.id.clone()))
        .chain([cancellation_port.id.clone(), task_port.id.clone()])
        .collect::<BTreeSet<_>>();
    let action_host_port_ref = host_port(WAVE4_CAPABILITY_HOST_PORT_ID);
    let context_host_port_ref = host_port(WAVE4_CONTEXT_HOST_PORT_ID);
    let turn_middleware_host_port_ref = host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID);
    let lifecycle_host_port_ref = host_port(WAVE4_LIFECYCLE_HOST_PORT_ID);
    let mut host_port_bindings = Vec::new();
    if has_action_handler {
        declared_host_ports.insert(action_host_port_ref.id.clone());
        host_port_bindings.push(host_port_binding()?);
    }
    if has_context_factory {
        declared_host_ports.insert(context_host_port_ref.id.clone());
        host_port_bindings.push(context_host_port_binding()?);
    }
    if has_turn_middleware {
        declared_host_ports.insert(turn_middleware_host_port_ref.id.clone());
        host_port_bindings.push(turn_middleware_host_port_binding()?);
    }
    if has_lifecycle {
        declared_host_ports.insert(lifecycle_host_port_ref.id.clone());
        host_port_bindings.push(lifecycle_host_port_binding()?);
    }
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|error| error.to_string())?,
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
                PluginRegistrarOperation::BindHostPort,
                PluginRegistrarOperation::ContributeCapability,
            ]),
            declared_capability_ids: spec
                .capabilities
                .iter()
                .map(|capability| CapabilityId::from(capability.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema)
                    .map_err(|error| error.to_string())?,
                config_revision: 1,
                value: empty_object(),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: host_port_bindings,
            typed_command_ports,
            domain_outbox_ports,
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
        },
    };

    let mut registration = PluginRegistration::new(metadata);
    for capability in spec.capabilities.iter().copied() {
        if capability.effect_class.is_none() {
            continue;
        }
        registration
            .add_capability_handler(
                CapabilityId::from(capability.id),
                Arc::new(Wave4CapabilityHandler {
                    capability_id: CapabilityId::from(capability.id),
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    for capability in spec.capabilities.iter().copied() {
        let capability_id = CapabilityId::from(capability.id);
        match capability.kind {
            CapabilityKind::ContextContributor => registration
                .add_capability_context_factory(
                    capability_id.clone(),
                    Arc::new(Wave4CapabilityContextFactory {
                        capability_id,
                        requirements: capability.requirements,
                        host_port: Arc::clone(&context_host_port),
                    }),
                )
                .map_err(|error| error.to_string())?,
            CapabilityKind::TurnMiddleware => registration
                .add_capability_context_factory(
                    capability_id.clone(),
                    Arc::new(Wave4CapabilityTurnMiddlewareFactory {
                        capability_id,
                        requirements: capability.requirements,
                        host_port: Arc::clone(&turn_middleware_host_port),
                    }),
                )
                .map_err(|error| error.to_string())?,
            CapabilityKind::ResourceProvider => registration
                .add_capability_resource_factory(
                    capability_id.clone(),
                    Arc::new(Wave4UnavailableResourceFactory { capability_id }),
                )
                .map_err(|error| error.to_string())?,
            _ => {}
        }
    }
    Ok(registration)
}

fn conversation_module_registration(
    spec: &PackageSpec,
    modules: &[ConversationModuleSpec],
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
    turn_middleware_host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
) -> Result<PluginRegistration, String> {
    let package = package_ref(spec.id);
    let config_schema = object_schema(false);
    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: localized(spec.display_name, spec.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", spec.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        }
        .into(),
        contributions: PackageContributions {
            capabilities: modules
                .iter()
                .map(|module| conversation_module_manifest(&package, module))
                .collect::<Result<Vec<_>, _>>()?,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
        },
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: spec.id.to_owned(),
        source_digest: None,
    };
    let mount_id = AgentModuleId::from(spec.mount_id);
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount_id.clone(),
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let action_port = host_port(WAVE4_CAPABILITY_HOST_PORT_ID);
    let context_port = host_port(WAVE4_CONTEXT_HOST_PORT_ID);
    let turn_middleware_port = host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID);
    let has_context = modules
        .iter()
        .any(|module| matches!(module.scene.map(|scene| scene.kind), Some(ConversationSceneKind::Context)));
    let has_turn_middleware = modules.iter().any(|module| {
        matches!(
            module.scene.map(|scene| scene.kind),
            Some(ConversationSceneKind::TurnMiddleware)
        )
    });
    let typed_command_ports = spec
        .ports
        .command_ports
        .iter()
        .map(|id| command_port(id))
        .collect::<Result<Vec<_>, _>>()?;
    let domain_outbox_ports = spec
        .ports
        .outbox_ports
        .iter()
        .map(|id| outbox_port(id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut declared_host_ports = typed_command_ports
        .iter()
        .map(|port| port.port.id.clone())
        .chain(domain_outbox_ports.iter().map(|port| port.port.id.clone()))
        .chain([
            cancellation_port.id.clone(),
            task_port.id.clone(),
            action_port.id.clone(),
        ])
        .collect::<BTreeSet<_>>();
    let mut host_port_bindings = vec![host_port_binding()?];
    if has_context {
        declared_host_ports.insert(context_port.id.clone());
        host_port_bindings.push(context_host_port_binding()?);
    }
    if has_turn_middleware {
        declared_host_ports.insert(turn_middleware_port.id.clone());
        host_port_bindings.push(turn_middleware_host_port_binding()?);
    }
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|error| error.to_string())?,
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
                PluginRegistrarOperation::BindHostPort,
                PluginRegistrarOperation::ContributeCapability,
            ]),
            declared_capability_ids: modules
                .iter()
                .map(|module| CapabilityId::from(module.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema)
                    .map_err(|error| error.to_string())?,
                config_revision: 1,
                value: empty_object(),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: host_port_bindings,
            typed_command_ports,
            domain_outbox_ports,
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    for module in modules {
        let module_id = CapabilityId::from(module.id);
        registration
            .add_capability_handler(
                module_id.clone(),
                Arc::new(Wave4CapabilityHandler {
                    capability_id: module_id.clone(),
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| error.to_string())?;
        let Some(scene) = module.scene else {
            continue;
        };
        match scene.kind {
            ConversationSceneKind::Context => registration
                .add_capability_context_factory(
                    module_id,
                    Arc::new(Wave4CapabilityContextFactory {
                        capability_id: CapabilityId::from(scene.id),
                        requirements: scene.requirements,
                        host_port: Arc::clone(&context_host_port),
                    }),
                )
                .map_err(|error| error.to_string())?,
            ConversationSceneKind::TurnMiddleware => registration
                .add_capability_context_factory(
                    module_id,
                    Arc::new(Wave4CapabilityTurnMiddlewareFactory {
                        capability_id: CapabilityId::from(scene.id),
                        requirements: scene.requirements,
                        host_port: Arc::clone(&turn_middleware_host_port),
                    }),
                )
                .map_err(|error| error.to_string())?,
        }
    }
    Ok(registration)
}

fn conversation_module_manifest(
    package: &PackageRef,
    module: &ConversationModuleSpec,
) -> Result<CapabilityManifest, String> {
    let actions = module
        .actions
        .iter()
        .map(|action| {
            let input = action_input_schema(action.id);
            let output = object_schema(true);
            Ok(CapabilityActionDescriptor {
                action_id: ActionId::from(action.id),
                input_schema: schema_ref(action.id, "input", &input)?,
                output_schema: schema_ref(action.id, "output", &output)?,
                effect_class: action.effect_class,
                presentation: ToolPresentationKind::FunctionTool,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (context_schema_refs, scene_host_port, scene_requirements) = match module.scene {
        Some(scene) => {
            let scene_schema = context_output_schema(scene.id);
            let scene_schema_ref = schema_ref(scene.id, "context", &scene_schema)?;
            let host_port = match scene.kind {
                ConversationSceneKind::Context => host_port(WAVE4_CONTEXT_HOST_PORT_ID),
                ConversationSceneKind::TurnMiddleware => {
                    host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID)
                }
            };
            (vec![scene_schema_ref], Some(host_port), scene.requirements)
        }
        None => (Vec::new(), None, &[][..]),
    };
    let resource_kinds = module
        .actions
        .iter()
        .flat_map(|action| action.resource_kinds.iter().copied())
        .chain(scene_requirements.iter().map(|requirement| requirement.resource_kind))
        .map(ResourceKind::from)
        .collect();
    Ok(CapabilityManifest {
        id: CapabilityId::from(module.id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "module:{}",
            module.id
        )),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: localized(module.display_name, module.description),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_module_surface_declarations(
            AGENT_SURFACES.iter().copied(),
            [CapabilityConsumer::Agent],
            CapabilityAuthoringPolicy::Direct,
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: object_schema(false),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs,
            context_phase: if module.scene.is_some() {
                nomifun_agent_contracts::ContextContributionPhase::BeforeTurn
            } else {
                nomifun_agent_contracts::ContextContributionPhase::SessionStart
            },
            ui_slot: None,
            event_schema_refs: Vec::new(),
            resource_kinds,
            host_ports: std::iter::once(host_port(WAVE4_CAPABILITY_HOST_PORT_ID))
                .chain(scene_host_port)
                .collect(),
        },
    })
}

fn capability_manifest(
    package: &PackageRef,
    spec: CapabilitySpec,
) -> Result<CapabilityManifest, String> {
    let mut contributions = CapabilityContributions {
        resource_kinds: spec
            .resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect(),
        ..CapabilityContributions::default()
    };

    if let Some(effect_class) = spec.effect_class {
        let input_schema = action_input_schema(spec.id);
        let output_schema = object_schema(true);
        contributions.actions.push(CapabilityActionDescriptor {
            action_id: action_id_for(spec.id),
            input_schema: schema_ref(spec.id, "input", &input_schema)?,
            output_schema: schema_ref(spec.id, "output", &output_schema)?,
            effect_class,
            presentation: ToolPresentationKind::FunctionTool,
        });
        contributions
            .host_ports
            .push(host_port(WAVE4_CAPABILITY_HOST_PORT_ID));
    }

    match spec.kind {
        CapabilityKind::ContextContributor => {
            let schema = context_output_schema(spec.id);
            contributions
                .context_schema_refs
                .push(schema_ref(spec.id, "context", &schema)?);
            contributions
                .host_ports
                .push(host_port(WAVE4_CONTEXT_HOST_PORT_ID));
        }
        CapabilityKind::TurnMiddleware => {
            let schema = context_output_schema(spec.id);
            contributions
                .context_schema_refs
                .push(schema_ref(spec.id, "context", &schema)?);
            contributions
                .host_ports
                .push(host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID));
        }
        CapabilityKind::EventSource => {
            let schema = object_schema(true);
            contributions
                .event_schema_refs
                .push(schema_ref(spec.id, "event", &schema)?);
            contributions
                .host_ports
                .push(host_port(WAVE4_LIFECYCLE_HOST_PORT_ID));
        }
        CapabilityKind::Transport => {
            contributions
                .host_ports
                .push(host_port(WAVE4_LIFECYCLE_HOST_PORT_ID));
        }
        CapabilityKind::EventConsumer => {
            let schema = object_schema(true);
            contributions
                .event_schema_refs
                .push(schema_ref(spec.id, "event", &schema)?);
        }
        _ => {}
    }

    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "capability:{}",
            spec.id
        )),
        kind: spec.kind,
        package: package.clone(),
        display: localized(spec.display_name, spec.description),
        requires: internal_capability_dependencies(spec.id),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            AGENT_SURFACES.iter().copied(),
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: object_schema(false),
        contributions,
    })
}

fn internal_capability_dependencies(capability_id: &str) -> Vec<CapabilityRef> {
    let _ = capability_id;
    Vec::new()
}

fn action_id_for(capability_id: &str) -> ActionId {
    ActionId::from(format!("{capability_id}.invoke"))
}

fn descriptor<const N: usize>(
    slot_key: &'static str,
    resource_kind: &'static str,
    required: bool,
    operations: [&'static str; N],
    binding_policy: &'static str,
) -> TypedResourceDescriptor {
    TypedResourceDescriptor {
        slot_key,
        resource_kind: ResourceKind::from(resource_kind),
        required,
        operations: operations
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        binding_policy,
    }
}

/// Return the canonical action input schema for a Wave 4 capability.
///
/// Nomi-core and domain adapters use these exact bytes when materializing a
/// bundled Tool, so the manifest digest and the model-facing schema cannot
/// drift into two independent contracts.
pub fn action_input_schema(capability_id: &str) -> StrictJsonValue {
    let schema = match capability_id {
        CHANNEL_MESSAGING_REPLY_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "destination_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "message_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "text": { "type": "string", "minLength": 1, "maxLength": 16384 }
            },
            "required": ["destination_ref", "message_ref", "text"],
            "additionalProperties": false
        }),
        CHANNEL_MESSAGING_SEND_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "destination_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "text": { "type": "string", "minLength": 1, "maxLength": 16384 }
            },
            "required": ["destination_ref", "text"],
            "additionalProperties": false
        }),
        COMPANION_LEARN_ACTION_ID | COMPANION_EVOLVE_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "minLength": 1, "maxLength": 512 }
            },
            "additionalProperties": false
        }),
        ROBOT_VISION_ACTION_ID
        | ROBOT_DISPLAY_ACTION_ID
        | ROBOT_MOTION_ACTION_ID
        | ROBOT_DEVICE_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        COMPANION_MEMORY_RECALL_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "per_kind": { "type": "integer", "minimum": 1, "maximum": 20 },
                "char_budget": { "type": "integer", "minimum": 1, "maximum": 65536 }
            },
            "additionalProperties": false
        }),
        COMPANION_MEMORY_WRITE_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "minLength": 1, "maxLength": 64 },
                "content": { "type": "string", "minLength": 1, "maxLength": 65536 },
                "tags": { "type": "array", "maxItems": 64, "items": { "type": "string", "minLength": 1, "maxLength": 128 } }
            },
            "required": ["kind", "content"],
            "additionalProperties": false
        }),
        CUSTOMER_SERVICE_NOTES_READ_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "cs_note_id": { "type": "string" },
                "include_disabled": { "type": "boolean", "default": false },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
            },
            "additionalProperties": false
        }),
        CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "cs_note_id": { "type": "string" },
                "kind": { "type": "string" },
                "content": { "type": "string", "minLength": 1 },
                "aliases": { "type": "string" },
                "enabled": { "type": "boolean" }
            },
            "additionalProperties": false
        }),
        CUSTOMER_SERVICE_HANDOFF_ACTION_ID => serde_json::json!({
            "type": "object",
            "properties": {
                "cs_dialogue_id": { "type": "string" },
                "reason": { "type": "string", "minLength": 1, "maxLength": 4000 },
                "summary": { "type": "string", "maxLength": 12000 }
            },
            "required": ["cs_dialogue_id", "reason"],
            "additionalProperties": false
        }),
        _ => return object_schema(true),
    };
    StrictJsonValue(schema)
}

fn context_output_schema(capability_id: &str) -> StrictJsonValue {
    let schema = match capability_id {
        CUSTOMER_SERVICE_DIALOGUE => serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "const": "customer_service_dialogue" },
                "cs_agent_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "cs_dialogue_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "system_prompt": { "type": "string", "minLength": 1, "maxLength": 65536 },
                "knowledge_base_ids": {
                    "type": "array",
                    "maxItems": 256,
                    "items": { "type": "string", "minLength": 1, "maxLength": 512 }
                }
            },
            "required": ["kind", "cs_agent_id", "system_prompt", "knowledge_base_ids"],
            "additionalProperties": false
        }),
        COMPANION_PERSONA => serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "const": "companion_persona" },
                "companion_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "system_prompt": { "type": "string", "minLength": 1, "maxLength": 65536 }
            },
            "required": ["kind", "companion_id", "system_prompt"],
            "additionalProperties": false
        }),
        COMPANION_ROSTER => serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "const": "companion_roster" },
                "selected_companion_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "companions": {
                    "type": "array",
                    "maxItems": 256,
                    "items": {
                        "type": "object",
                        "properties": {
                            "companion_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                            "seq": { "type": "integer", "minimum": 1 },
                            "name": { "type": "string", "minLength": 1, "maxLength": 512 },
                            "character": { "type": "string", "minLength": 1, "maxLength": 512 }
                        },
                        "required": ["companion_id", "seq", "name", "character"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["kind", "selected_companion_id", "companions"],
            "additionalProperties": false
        }),
        _ => return object_schema(true),
    };
    StrictJsonValue(schema)
}

fn object_schema(additional_properties: bool) -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({
        "type": "object",
        "additionalProperties": additional_properties,
    }))
}

fn empty_object() -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({}))
}

fn schema_ref(
    capability_id: &str,
    facet: &str,
    schema: &StrictJsonValue,
) -> Result<CanonicalSchemaRef, String> {
    let digest = digest_payload(schema).map_err(|error| error.to_string())?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{capability_id}/{facet}@1#{}",
        digest.as_ref()
    )))
}

fn host_port(id: &str) -> nomifun_agent_contracts::HostPortRef {
    nomifun_agent_contracts::HostPortRef {
        id: nomifun_agent_contracts::HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = object_schema(true);
    let response_schema = object_schema(true);
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE4_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(
            WAVE4_CAPABILITY_HOST_PORT_ID,
            "request",
            &request_schema,
        )?,
        response_schema: schema_ref(
            WAVE4_CAPABILITY_HOST_PORT_ID,
            "response",
            &response_schema,
        )?,
    })
}

fn context_host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = object_schema(true);
    let response_schema = object_schema(true);
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE4_CONTEXT_HOST_PORT_ID),
        request_schema: schema_ref(
            WAVE4_CONTEXT_HOST_PORT_ID,
            "request",
            &request_schema,
        )?,
        response_schema: schema_ref(
            WAVE4_CONTEXT_HOST_PORT_ID,
            "response",
            &response_schema,
        )?,
    })
}

fn turn_middleware_host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = object_schema(true);
    let response_schema = object_schema(true);
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID),
        request_schema: schema_ref(
            WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID,
            "request",
            &request_schema,
        )?,
        response_schema: schema_ref(
            WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID,
            "response",
            &response_schema,
        )?,
    })
}

fn lifecycle_host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = object_schema(true);
    let response_schema = object_schema(true);
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE4_LIFECYCLE_HOST_PORT_ID),
        request_schema: schema_ref(
            WAVE4_LIFECYCLE_HOST_PORT_ID,
            "request",
            &request_schema,
        )?,
        response_schema: schema_ref(
            WAVE4_LIFECYCLE_HOST_PORT_ID,
            "response",
            &response_schema,
        )?,
    })
}

fn command_port(id: &str) -> Result<TypedCommandPortDescriptor, String> {
    let command_schema = object_schema(true);
    let receipt_schema = object_schema(true);
    Ok(TypedCommandPortDescriptor {
        port: host_port(id),
        command_schema: schema_ref(id, "command", &command_schema)?,
        receipt_schema: schema_ref(id, "receipt", &receipt_schema)?,
    })
}

fn outbox_port(id: &str) -> Result<DomainOutboxPortDescriptor, String> {
    let event_schema = object_schema(true);
    let cursor_schema = object_schema(true);
    Ok(DomainOutboxPortDescriptor {
        port: host_port(id),
        event_schema: schema_ref(id, "event", &event_schema)?,
        cursor_schema: schema_ref(id, "cursor", &cursor_schema)?,
    })
}

fn package_ref(package_id: &str) -> PackageRef {
    PackageRef {
        id: PackageId::from(package_id),
        version: VersionString::from(PACKAGE_VERSION),
    }
}

fn localized(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

struct Wave4CapabilityContextFactory {
    capability_id: CapabilityId,
    requirements: &'static [ResourceRequirement],
    host_port: Arc<dyn Wave4ContextHostPort>,
}

impl CapabilityContextContributionFactory for Wave4CapabilityContextFactory {
    fn contribute<'life0, 'async_trait>(
        &'life0 self,
        request: CapabilityContextContributionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ContextContributionResult, KernelError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            validate_resource_bindings(
                &self.capability_id,
                &request.context.principal.principal_id,
                self.requirements,
                &request.context.resource_bindings,
            )?;
            let host_request = Wave4ContextHostRequest {
                principal: request.context.principal,
                agent_session_id: request.context.agent_session_id,
                operation_id: request.context.operation_id,
                correlation_id: request.context.correlation_id,
                resolved_snapshot_ref: request.context.resolved_snapshot_ref,
                registry_generation: request.context.registry_generation,
                registry_digest: request.context.registry_digest,
                capability_id: self.capability_id.clone(),
                state_scope_key: request.context.state_scope_key,
                resource_bindings: request.context.resource_bindings,
                schema_ref: request.schema_ref,
            };
            host_request
                .validate()
                .map_err(wave4_host_error_to_kernel)?;
            self.host_port
                .contribute(host_request)
                .await
                .map(|value| ContextContributionResult { value })
                .map_err(wave4_host_error_to_kernel)
        })
    }
}

struct Wave4CapabilityTurnMiddlewareFactory {
    capability_id: CapabilityId,
    requirements: &'static [ResourceRequirement],
    host_port: Arc<dyn Wave4TurnMiddlewareHostPort>,
}

impl CapabilityContextContributionFactory for Wave4CapabilityTurnMiddlewareFactory {
    fn contribute<'life0, 'async_trait>(
        &'life0 self,
        request: CapabilityContextContributionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ContextContributionResult, KernelError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            validate_resource_bindings(
                &self.capability_id,
                &request.context.principal.principal_id,
                self.requirements,
                &request.context.resource_bindings,
            )?;
            let nomifun_agent_contracts::ContextContributionInput::BeforeTurn { turn } =
                request.input
            else {
                return Err(KernelError::CapabilityExecution {
                    reason: format!(
                        "TurnMiddleware {} requires canonical BeforeTurn input",
                        self.capability_id.as_ref()
                    ),
                });
            };
            let host_request = Wave4TurnMiddlewareHostRequest {
                principal: request.context.principal,
                agent_session_id: request.context.agent_session_id,
                operation_id: request.context.operation_id,
                correlation_id: request.context.correlation_id,
                resolved_snapshot_ref: request.context.resolved_snapshot_ref,
                registry_generation: request.context.registry_generation,
                registry_digest: request.context.registry_digest,
                capability_id: self.capability_id.clone(),
                state_scope_key: request.context.state_scope_key,
                resource_bindings: request.context.resource_bindings,
                schema_ref: request.schema_ref,
                turn_input: StrictJsonValue(serde_json::json!({
                    "source_message_id": turn.source_message_id,
                    "text": turn.text,
                    "image_media_types": turn.image_media_types,
                    "cs_dialogue_id": turn.cs_dialogue_id,
                })),
            };
            host_request
                .validate()
                .map_err(wave4_host_error_to_kernel)?;
            self.host_port
                .apply(host_request)
                .await
                .map(|value| ContextContributionResult { value: Some(value) })
                .map_err(wave4_host_error_to_kernel)
        })
    }
}

struct Wave4UnavailableResourceFactory {
    capability_id: CapabilityId,
}

impl CapabilityResourceProviderFactory for Wave4UnavailableResourceFactory {
    fn acquire<'life0, 'async_trait>(
        &'life0 self,
        _request: CapabilityResourceProviderRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceProviderResult, KernelError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            Err(KernelError::CapabilityExecution {
                reason: format!(
                    "Wave 4 Resource capability {} has no configured resource owner",
                    self.capability_id.as_ref()
                ),
            })
        })
    }
}

struct Wave4CapabilityHandler {
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave4HostPort>,
}

impl CapabilityHandler for Wave4CapabilityHandler {
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
            if context.capability_id != self.capability_id {
                return Err(KernelError::ActionNotDeclared {
                    capability_id: context.capability_id,
                    action_id: context.action_id,
                });
            }
            let requirements = action_requirements(
                self.capability_id.as_ref(),
                context.action_id.as_ref(),
            )
            .ok_or_else(|| KernelError::ActionNotDeclared {
                capability_id: context.capability_id.clone(),
                action_id: context.action_id.clone(),
            })?;
            let operation = operation_from_action_input(
                &self.capability_id,
                &context.action_id,
                input,
            )?;

            validate_resource_bindings(
                &self.capability_id,
                &context.principal.principal_id,
                requirements,
                &context.resource_bindings,
            )?;
            let request = Wave4HostRequest {
                context: Wave4HostContext {
                    principal: context.principal,
                    agent_session_id: context.agent_session_id,
                    operation_id: context.operation_id,
                    idempotency_key: context.idempotency_key,
                    correlation_id: context.correlation_id,
                    resolved_snapshot_ref: context.resolved_snapshot_ref,
                    registry_generation: context.registry_generation,
                    capability_id: self.capability_id.clone(),
                    action_id: context.action_id.clone(),
                    state_scope_key: context.state_scope_key,
                    resource_bindings: context.resource_bindings,
                },
                operation,
            };
            request
                .validate()
                .map_err(wave4_host_error_to_kernel)?;

            self.host_port
                .invoke(request)
                .await
                .map_err(wave4_host_error_to_kernel)
        })
    }
}

/// Convert a canonical capability ID and its object payload into the only
/// typed operation variant accepted by the host port.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave4CapabilityOperation, KernelError> {
    operation_from_action_input(
        capability_id,
        &canonical_action_id(capability_id.as_ref()).ok_or_else(|| {
            KernelError::CapabilityExecution {
                reason: format!("{} has no single legacy Action", capability_id.as_ref()),
            }
        })?,
        input,
    )
}

pub fn operation_from_action_input(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    input: StrictJsonValue,
) -> Result<Wave4CapabilityOperation, KernelError> {
    let operation = match (capability_id.as_ref(), action_id.as_ref()) {
        (CHANNEL_MESSAGING_MODULE_ID, CHANNEL_MESSAGING_REPLY_ACTION_ID) => {
            Wave4CapabilityOperation::ChannelReply { input }
        }
        (CHANNEL_MESSAGING_MODULE_ID, CHANNEL_MESSAGING_SEND_ACTION_ID) => {
            Wave4CapabilityOperation::ChannelSend { input }
        }
        (COMPANION_MODULE_ID, COMPANION_LEARN_ACTION_ID) => {
            Wave4CapabilityOperation::CompanionLearn { input }
        }
        (COMPANION_MODULE_ID, COMPANION_EVOLVE_ACTION_ID) => {
            Wave4CapabilityOperation::CompanionEvolve { input }
        }
        (COMPANION_MEMORY_MODULE_ID, COMPANION_MEMORY_RECALL_ACTION_ID) => {
            Wave4CapabilityOperation::CompanionMemoryRecall { input }
        }
        (COMPANION_MEMORY_MODULE_ID, COMPANION_MEMORY_WRITE_ACTION_ID) => {
            Wave4CapabilityOperation::CompanionMemoryWrite { input }
        }
        (CUSTOMER_SERVICE_MODULE_ID, CUSTOMER_SERVICE_NOTES_READ_ACTION_ID) => {
            Wave4CapabilityOperation::CustomerServiceNotesRead { input }
        }
        (CUSTOMER_SERVICE_MODULE_ID, CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID) => {
            Wave4CapabilityOperation::CustomerServiceNotesWrite { input }
        }
        (CUSTOMER_SERVICE_MODULE_ID, CUSTOMER_SERVICE_HANDOFF_ACTION_ID) => {
            Wave4CapabilityOperation::CustomerServiceHandoff { input }
        }
        (ROBOT_MODULE_ID, ROBOT_VISION_ACTION_ID) => {
            Wave4CapabilityOperation::RobotVision { input }
        }
        (ROBOT_MODULE_ID, ROBOT_DISPLAY_ACTION_ID) => {
            Wave4CapabilityOperation::RobotDisplay { input }
        }
        (ROBOT_MODULE_ID, ROBOT_MOTION_ACTION_ID) => {
            Wave4CapabilityOperation::RobotMotion { input }
        }
        (ROBOT_MODULE_ID, ROBOT_DEVICE_ACTION_ID) => {
            Wave4CapabilityOperation::RobotDevice { input }
        }
        (capability_id, action_id) => {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{capability_id}/{action_id} does not expose an action host operation"
                ),
            });
        }
    };
    if !operation.input().0.is_object() {
        return Err(wave4_host_error_to_kernel(Wave4HostPortError::invalid_request(
            format!("{} input must be a JSON object", action_id.as_ref()),
        )));
    }
    Ok(operation)
}

fn validate_resource_bindings(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &[TypedResourceBinding],
) -> Result<(), KernelError> {
    validate_resource_bindings_contract(capability_id, principal_id, requirements, bindings)
        .map_err(|error| {
            if error.code == WAVE4_RESOURCE_OWNER_MISMATCH {
                let binding_id = bindings
                    .iter()
                    .find(|binding| binding.owner_id != principal_id)
                    .map(|binding| binding.binding_id.clone())
                    .unwrap_or_else(|| ResourceBindingId::from("unknown"));
                KernelError::ResourceOwnerMismatch { binding_id }
            } else if error.code == WAVE4_RESOURCE_NOT_BOUND {
                let missing_kind = requirements
                    .iter()
                    .find(|requirement| {
                        !bindings.iter().any(|binding| {
                            binding.resource_kind.as_ref() == requirement.resource_kind
                                && binding.operations.contains(requirement.operation)
                        })
                    })
                    .map(|requirement| requirement.resource_kind)
                    .unwrap_or("missing");
                KernelError::ResourceBindingMissing {
                    binding_id: ResourceBindingId::from(missing_kind),
                }
            } else {
                wave4_host_error_to_kernel(error)
            }
        })
}

fn wave4_host_error_to_kernel(error: Wave4HostPortError) -> KernelError {
    KernelError::capability_execution_failed(error.code, error.message)
}

fn validate_host_context(context: &Wave4HostContext) -> Result<(), Wave4HostPortError> {
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
    if let Some((field, _)) = fields
        .iter()
        .find(|(_, value)| value.trim().is_empty())
    {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{field} must be non-empty"
        )));
    }
    if context.registry_generation == 0 {
        return Err(Wave4HostPortError::invalid_request(
            "registry_generation must identify a published generation",
        ));
    }
    Ok(())
}

fn validate_resource_bindings_contract(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &[TypedResourceBinding],
) -> Result<(), Wave4HostPortError> {
    if principal_id.trim().is_empty() {
        return Err(Wave4HostPortError::invalid_request(
            "principal.principal_id must be non-empty",
        ));
    }

    let expected_kinds = requirements
        .iter()
        .map(|requirement| ResourceKind::from(requirement.resource_kind))
        .collect::<BTreeSet<_>>();
    let declared_operations = typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect::<BTreeMap<_, _>>();
    let mut seen_binding_ids = BTreeSet::new();
    let mut seen_resource_kinds = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().trim().is_empty()
            || binding.resource_kind.as_ref().trim().is_empty()
            || binding.resource_id.as_ref().trim().is_empty()
            || binding.owner_id.trim().is_empty()
        {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
                "{} requires non-empty binding, resource kind, resource ID, and owner ID",
                capability_id.as_ref()
            )));
        }
        if !seen_binding_ids.insert(binding.binding_id.clone()) {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource binding {}",
                capability_id.as_ref(),
                binding.binding_id.as_ref()
            )));
        }
        if binding.owner_id != principal_id {
            return Err(Wave4HostPortError::resource_owner_mismatch(format!(
                "resource binding {} belongs to {}, not {}",
                binding.binding_id.as_ref(),
                binding.owner_id,
                principal_id
            )));
        }
        if !seen_resource_kinds.insert(binding.resource_kind.clone()) {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        if !expected_kinds.contains(&binding.resource_kind) {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
                "{} received unexpected resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        let Some(allowed_operations) = declared_operations.get(&binding.resource_kind) else {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave4HostPortError::resource_not_bound(format!(
                "{} is missing resource kind {}",
                capability_id.as_ref(),
                requirement.resource_kind
            )));
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(Wave4HostPortError::resource_not_bound(format!(
                "{} requires operation {} on {}",
                capability_id.as_ref(),
                requirement.operation,
                requirement.resource_kind
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
