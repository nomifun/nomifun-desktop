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
    CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind,
    CapabilityManifest,
    CanonicalSchemaRef, CancellationDescriptor, CorrelationId, DigestHex,
    DeclaredServiceViewDescriptor, DomainOutboxPortDescriptor, EffectClass,
    HostPortBindingDescriptor, IdempotencyKey,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor, PluginMountId,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    StrictJsonValue, ToolPresentationKind, TypedCommandPortDescriptor, TypedResourceBinding,
    TypedResourceBindings, ValidatedPluginConfig, VersionString,
    capability_surface_declarations, digest_payload,
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

pub const CHANNEL_RECEIVE: &str = "channel.receive";
pub const CHANNEL_REPLY: &str = "channel.reply";
pub const CHANNEL_SEND: &str = "channel.send";
pub const CHANNEL_PAIRING: &str = "channel.pairing";
pub const CHANNEL_GROUP_POLICY: &str = "channel.group_policy";
pub const COMPANION_EVOLVE: &str = "companion.evolve";
pub const COMPANION_LEARN: &str = "companion.learn";
pub const COMPANION_PERSONA: &str = "companion.persona";
pub const COMPANION_ROSTER: &str = "companion.roster";
pub const CUSTOMER_SERVICE_DIALOGUE: &str = "customer_service.dialogue";
pub const CUSTOMER_SERVICE_NOTES_READ: &str = "customer_service.notes.read";
pub const CUSTOMER_SERVICE_NOTES_WRITE: &str = "customer_service.notes.write";
pub const CUSTOMER_SERVICE_HANDOFF: &str = "customer_service.handoff";
pub const NOTIFICATION_WEBHOOK: &str = "notification.webhook";
pub const NOTIFICATION_DESKTOP: &str = "notification.desktop";
pub const ROBOT_LINK: &str = "robot.link";
pub const ROBOT_AUDIO: &str = "robot.audio";
pub const ROBOT_DEVICE_TOOLS: &str = "robot.device_tools";
pub const ROBOT_DISPLAY: &str = "robot.display";
pub const ROBOT_MOTION: &str = "robot.motion";
pub const ROBOT_VISION: &str = "robot.vision";

pub const CHANNEL_REPLY_ACTION: &str = "channel.reply.invoke";
pub const CHANNEL_SEND_ACTION: &str = "channel.send.invoke";
pub const COMPANION_EVOLVE_ACTION: &str = "companion.evolve.invoke";
pub const COMPANION_LEARN_ACTION: &str = "companion.learn.invoke";
pub const CUSTOMER_SERVICE_NOTES_READ_ACTION: &str = "customer_service.notes.read.invoke";
pub const CUSTOMER_SERVICE_NOTES_WRITE_ACTION: &str = "customer_service.notes.write.invoke";
pub const CUSTOMER_SERVICE_HANDOFF_ACTION: &str = "customer_service.handoff.invoke";
pub const ROBOT_DEVICE_TOOLS_ACTION: &str = "robot.device_tools.invoke";
pub const ROBOT_DISPLAY_ACTION: &str = "robot.display.invoke";
pub const ROBOT_MOTION_ACTION: &str = "robot.motion.invoke";

pub const PACKAGE_IDS: [&str; 5] = [
    CHANNEL_PACKAGE_ID,
    COMPANION_PACKAGE_ID,
    CUSTOMER_SERVICE_PACKAGE_ID,
    ROBOT_PACKAGE_ID,
    NOTIFICATION_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 5] = PACKAGE_IDS;

/// Family spelling used by the C7 deletion contract.
///
/// The frozen first-party catalog spells the two multiword IDs with
/// underscores.  [`canonical_capability_id`] makes that normalization
/// explicit instead of creating duplicate aliases.
pub const TARGET_CAPABILITY_FAMILIES: [&str; 14] = [
    "channel.receive",
    "channel.reply",
    "channel.send",
    "companion.evolve",
    "companion.learn",
    "companion.persona",
    "customer-service.dialogue",
    "customer-service.handoff",
    "notification.webhook",
    "robot.audio",
    "robot.device-tools",
    "robot.display",
    "robot.motion",
    "robot.vision",
];

/// Exact canonical capability IDs contributed by the five target packages.
///
/// This is intentionally the full checked-in target-package inventory, not
/// only the deletion-contract family subset.
pub const TARGET_CAPABILITY_IDS: [&str; 21] = [
    CHANNEL_RECEIVE,
    CHANNEL_REPLY,
    CHANNEL_SEND,
    CHANNEL_PAIRING,
    CHANNEL_GROUP_POLICY,
    COMPANION_PERSONA,
    COMPANION_ROSTER,
    COMPANION_LEARN,
    COMPANION_EVOLVE,
    CUSTOMER_SERVICE_DIALOGUE,
    CUSTOMER_SERVICE_NOTES_READ,
    CUSTOMER_SERVICE_NOTES_WRITE,
    CUSTOMER_SERVICE_HANDOFF,
    ROBOT_LINK,
    ROBOT_AUDIO,
    ROBOT_VISION,
    ROBOT_DISPLAY,
    ROBOT_MOTION,
    ROBOT_DEVICE_TOOLS,
    NOTIFICATION_WEBHOOK,
    NOTIFICATION_DESKTOP,
];
pub const CAPABILITY_IDS: [&str; 21] = TARGET_CAPABILITY_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 21] = TARGET_CAPABILITY_IDS;

const AGENT_SURFACES: &[&str] = &["desktop", "headless", "remote", "web"];
const CHANNEL_RESOURCE: &[&str] = &[CHANNEL_RESOURCE_KIND];
const COMPANION_RESOURCE: &[&str] = &[COMPANION_RESOURCE_KIND];
const COMPANION_MEMORY_RESOURCE: &[&str] = &[COMPANION_MEMORY_RESOURCE_KIND];
const CUSTOMER_RESOURCE: &[&str] = &[CUSTOMER_RESOURCE_KIND];
const ROBOT_RESOURCE: &[&str] = &[ROBOT_RESOURCE_KIND];

#[derive(Clone, Copy)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

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
    CustomerServiceNotesRead { input: StrictJsonValue },
    CustomerServiceNotesWrite { input: StrictJsonValue },
    CustomerServiceHandoff { input: StrictJsonValue },
    RobotDisplay { input: StrictJsonValue },
    RobotMotion { input: StrictJsonValue },
    RobotDeviceTools { input: StrictJsonValue },
}

impl Wave4CapabilityOperation {
    /// Return the canonical capability identity fixed by this typed variant.
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::ChannelReply { .. } => CHANNEL_REPLY,
            Self::ChannelSend { .. } => CHANNEL_SEND,
            Self::CompanionLearn { .. } => COMPANION_LEARN,
            Self::CompanionEvolve { .. } => COMPANION_EVOLVE,
            Self::CustomerServiceNotesRead { .. } => CUSTOMER_SERVICE_NOTES_READ,
            Self::CustomerServiceNotesWrite { .. } => CUSTOMER_SERVICE_NOTES_WRITE,
            Self::CustomerServiceHandoff { .. } => CUSTOMER_SERVICE_HANDOFF,
            Self::RobotDisplay { .. } => ROBOT_DISPLAY,
            Self::RobotMotion { .. } => ROBOT_MOTION,
            Self::RobotDeviceTools { .. } => ROBOT_DEVICE_TOOLS,
        })
    }

    /// Return the canonical action identity paired with this operation.
    pub fn action_id(&self) -> ActionId {
        action_id_for(self.capability_id().as_ref())
    }

    /// Return the first-party owner domain for the operation.
    pub fn owner_domain(&self) -> Wave4OwnerDomain {
        match self {
            Self::ChannelReply { .. } | Self::ChannelSend { .. } => Wave4OwnerDomain::Channel,
            Self::CompanionLearn { .. } | Self::CompanionEvolve { .. } => Wave4OwnerDomain::Companion,
            Self::CustomerServiceNotesRead { .. }
            | Self::CustomerServiceNotesWrite { .. }
            | Self::CustomerServiceHandoff { .. } => Wave4OwnerDomain::CustomerService,
            Self::RobotDisplay { .. }
            | Self::RobotMotion { .. }
            | Self::RobotDeviceTools { .. } => Wave4OwnerDomain::Robot,
        }
    }

    fn input(&self) -> &StrictJsonValue {
        match self {
            Self::ChannelReply { input }
            | Self::ChannelSend { input }
            | Self::CompanionLearn { input }
            | Self::CompanionEvolve { input }
            | Self::CustomerServiceNotesRead { input }
            | Self::CustomerServiceNotesWrite { input }
            | Self::CustomerServiceHandoff { input }
            | Self::RobotDisplay { input }
            | Self::RobotMotion { input }
            | Self::RobotDeviceTools { input } => input,
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
        let Some(spec) = find_capability(capability_id.as_ref()) else {
            return Err(Wave4HostPortError::invalid_request(format!(
                "unknown Wave 4 capability {}",
                capability_id.as_ref()
            )));
        };
        if spec.effect_class.is_none() {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "{} is transport/context/event owned and has no Agent action host operation",
                capability_id.as_ref()
            )));
        }

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
            spec.requirements,
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
        let Some(spec) = find_capability(self.capability_id.as_ref()) else {
            return Err(Wave4HostPortError::invalid_request(format!(
                "unknown Wave 4 Context capability {}",
                self.capability_id.as_ref()
            )));
        };
        if spec.kind != CapabilityKind::ContextContributor {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "{} is not a Context contribution",
                self.capability_id.as_ref()
            )));
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
                "{} received a non-canonical Context schema",
                self.capability_id.as_ref()
            )));
        }
        validate_resource_bindings_contract(
            &self.capability_id,
            &self.principal.principal_id,
            spec.requirements,
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
        let Some(spec) = find_capability(self.capability_id.as_ref()) else {
            return Err(Wave4HostPortError::invalid_request(format!(
                "unknown Wave 4 TurnMiddleware capability {}",
                self.capability_id.as_ref()
            )));
        };
        if spec.kind != CapabilityKind::TurnMiddleware {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "{} is not a TurnMiddleware capability",
                self.capability_id.as_ref()
            )));
        }
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
            spec.requirements,
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
    pub robot: Option<Arc<dyn Wave4ContextHostPort>>,
}

impl Wave4ContextOwnerBindings {
    pub fn with_companion(mut self, owner: Arc<dyn Wave4ContextHostPort>) -> Self {
        self.companion = Some(owner);
        self
    }

    pub fn with_robot(mut self, owner: Arc<dyn Wave4ContextHostPort>) -> Self {
        self.robot = Some(owner);
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
            ROBOT_VISION => self.bindings.robot.clone(),
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

const CHANNEL_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec {
        id: CHANNEL_RECEIVE,
        kind: CapabilityKind::EventSource,
        display_name: "Channel receive",
        description: "Route an inbound channel event to a canonical AgentSession.",
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "receive",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: CHANNEL_REPLY,
        kind: CapabilityKind::Tool,
        display_name: "Channel reply",
        description: "Reply through the selected typed channel resource.",
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "reply",
        }],
        effect_class: Some(EffectClass::ExternalTransmit),
    },
    CapabilitySpec {
        id: CHANNEL_SEND,
        kind: CapabilityKind::Tool,
        display_name: "Channel send",
        description: "Send an outbound message through the selected typed channel resource.",
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "send",
        }],
        effect_class: Some(EffectClass::ExternalTransmit),
    },
    CapabilitySpec {
        id: CHANNEL_PAIRING,
        kind: CapabilityKind::Transport,
        display_name: "Channel pairing",
        description: "Expose the transport-owned channel pairing boundary.",
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "manage",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: CHANNEL_GROUP_POLICY,
        kind: CapabilityKind::TurnMiddleware,
        display_name: "Channel group policy",
        description: "Apply the owning channel's group policy to a turn.",
        resource_kinds: CHANNEL_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CHANNEL_RESOURCE_KIND,
            operation: "manage",
        }],
        effect_class: None,
    },
];

const COMPANION_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec {
        id: COMPANION_PERSONA,
        kind: CapabilityKind::ContextContributor,
        display_name: "Companion persona",
        description: "Provide the selected Companion persona as Agent context.",
        resource_kinds: COMPANION_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: COMPANION_ROSTER,
        kind: CapabilityKind::ContextContributor,
        display_name: "Companion roster",
        description: "Provide the available Companion roster as typed context.",
        resource_kinds: COMPANION_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: COMPANION_LEARN,
        kind: CapabilityKind::Tool,
        display_name: "Companion learn",
        description: "Submit a bounded learning command for Companion memory.",
        resource_kinds: COMPANION_MEMORY_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_MEMORY_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: Some(EffectClass::WriteDurable),
    },
    CapabilitySpec {
        id: COMPANION_EVOLVE,
        kind: CapabilityKind::Tool,
        display_name: "Companion evolve",
        description: "Submit a bounded evolution command for Companion memory.",
        resource_kinds: COMPANION_MEMORY_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_MEMORY_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: Some(EffectClass::WriteDurable),
    },
];

const CUSTOMER_SERVICE_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec {
        id: CUSTOMER_SERVICE_DIALOGUE,
        kind: CapabilityKind::TurnMiddleware,
        display_name: "Customer service dialogue",
        description: "Route a turn through the selected customer resource.",
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: CUSTOMER_SERVICE_NOTES_READ,
        kind: CapabilityKind::Tool,
        display_name: "Customer service notes read",
        description: "Read notes owned by the selected customer resource.",
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: Some(EffectClass::ReadSensitive),
    },
    CapabilitySpec {
        id: CUSTOMER_SERVICE_NOTES_WRITE,
        kind: CapabilityKind::Tool,
        display_name: "Customer service notes write",
        description: "Write notes owned by the selected customer resource.",
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: Some(EffectClass::WriteDurable),
    },
    CapabilitySpec {
        id: CUSTOMER_SERVICE_HANDOFF,
        kind: CapabilityKind::Tool,
        display_name: "Customer service handoff",
        description: "Submit a typed handoff command for the selected customer.",
        resource_kinds: CUSTOMER_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: CUSTOMER_RESOURCE_KIND,
            operation: "write",
        }],
        effect_class: Some(EffectClass::ExternalTransmit),
    },
];

const ROBOT_CAPABILITIES: [CapabilitySpec; 6] = [
    CapabilitySpec {
        id: ROBOT_LINK,
        kind: CapabilityKind::ResourceProvider,
        display_name: "Robot link",
        description: "Expose the selected Robot device-link resource boundary.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "link",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: ROBOT_AUDIO,
        kind: CapabilityKind::BackgroundService,
        display_name: "Robot audio",
        description: "Provide the selected Robot audio service boundary.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "audio",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: ROBOT_VISION,
        kind: CapabilityKind::ContextContributor,
        display_name: "Robot vision",
        description: "Provide selected Robot observations as typed context.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "vision",
        }],
        effect_class: None,
    },
    CapabilitySpec {
        id: ROBOT_DISPLAY,
        kind: CapabilityKind::Tool,
        display_name: "Robot display",
        description: "Submit a typed display command for the selected Robot.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "display",
        }],
        effect_class: Some(EffectClass::Physical),
    },
    CapabilitySpec {
        id: ROBOT_MOTION,
        kind: CapabilityKind::Tool,
        display_name: "Robot motion",
        description: "Submit a typed motion command for the selected Robot.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            operation: "motion",
        }],
        effect_class: Some(EffectClass::Physical),
    },
    CapabilitySpec {
        id: ROBOT_DEVICE_TOOLS,
        kind: CapabilityKind::Tool,
        display_name: "Robot device tools",
        description: "Submit a typed device-tool command for the selected Robot.",
        resource_kinds: ROBOT_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: ROBOT_RESOURCE_KIND,
            // Device tools are discovered and dispatched through the selected
            // device link.  The frozen Robot binding contract has no separate
            // `device_tools` operation.
            operation: "link",
        }],
        effect_class: Some(EffectClass::Physical),
    },
];

const NOTIFICATION_CAPABILITIES: [CapabilitySpec; 2] = [
    CapabilitySpec {
        id: NOTIFICATION_WEBHOOK,
        kind: CapabilityKind::EventConsumer,
        display_name: "Webhook notification",
        description: "Consume an owning-domain event for webhook delivery.",
        resource_kinds: &[],
        requirements: &[],
        effect_class: None,
    },
    CapabilitySpec {
        id: NOTIFICATION_DESKTOP,
        kind: CapabilityKind::EventConsumer,
        display_name: "Desktop notification",
        description: "Consume an owning-domain event for desktop notification projection.",
        resource_kinds: &[],
        requirements: &[],
        effect_class: None,
    },
];

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
        capabilities: &CHANNEL_CAPABILITIES,
        ports: CHANNEL_PORTS,
    },
    PackageSpec {
        id: COMPANION_PACKAGE_ID,
        mount_id: "domain-companion",
        display_name: "Companion",
        description: "Bundled Companion persona, learning, and evolution capabilities.",
        capabilities: &COMPANION_CAPABILITIES,
        ports: COMPANION_PORTS,
    },
    PackageSpec {
        id: CUSTOMER_SERVICE_PACKAGE_ID,
        mount_id: "domain-customer-service",
        display_name: "Customer Service",
        description: "Bundled customer dialogue and handoff capabilities.",
        capabilities: &CUSTOMER_SERVICE_CAPABILITIES,
        ports: CUSTOMER_SERVICE_PORTS,
    },
    PackageSpec {
        id: ROBOT_PACKAGE_ID,
        mount_id: "domain-robot",
        display_name: "Robot",
        description: "Bundled Robot media, display, motion, and device capabilities.",
        capabilities: &ROBOT_CAPABILITIES,
        ports: ROBOT_PORTS,
    },
    PackageSpec {
        id: NOTIFICATION_PACKAGE_ID,
        mount_id: "domain-notification",
        display_name: "Notification",
        description: "Bundled webhook event consumption.",
        capabilities: &NOTIFICATION_CAPABILITIES,
        ports: NOTIFICATION_PORTS,
    },
];

/// Resolve a deletion-contract family to its stable catalog ID.
pub fn canonical_capability_id(family: &str) -> Option<CapabilityId> {
    let normalized = family.replace('-', "_");
    TARGET_CAPABILITY_IDS
        .iter()
        .find(|candidate| **candidate == family || **candidate == normalized)
        .map(|candidate| CapabilityId::from(*candidate))
}

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
            ["audio", "display", "link", "motion", "vision"],
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
            ["audio", "display", "link", "motion", "vision"],
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
    find_capability(capability_id).map(|spec| {
        spec.resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
}

/// Resolve the only action identity that may be used for a capability.
pub fn canonical_action_id(capability_id: &str) -> Option<ActionId> {
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
    PACKAGE_SPECS
        .iter()
        .map(|spec| {
            registration_for(
                spec,
                Arc::clone(&action_host_port),
                Arc::clone(&context_host_port),
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
    registration_for(
        &PACKAGE_SPECS[0],
        action_host_port,
        context_host_port,
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
    registration_for(
        &PACKAGE_SPECS[2],
        action_host_port,
        context_host_port,
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
    )
}

pub fn notification_registration() -> Result<PluginRegistration, String> {
    registration_for(
        &PACKAGE_SPECS[4],
        unconfigured_host_port(),
        unconfigured_context_host_port(),
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

fn registration_for(
    spec: &PackageSpec,
    action_host_port: Arc<dyn Wave4HostPort>,
    context_host_port: Arc<dyn Wave4ContextHostPort>,
) -> Result<PluginRegistration, String> {
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
    let mount_id = PluginMountId::from(spec.mount_id);
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
        let action_id = action_id_for(capability.id);
        registration
            .add_capability_handler(
                CapabilityId::from(capability.id),
                Arc::new(Wave4CapabilityHandler {
                    capability_id: CapabilityId::from(capability.id),
                    action_id,
                    requirements: capability.requirements,
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
        version: VersionString::from(CONTRACT_VERSION),
        kind: spec.kind,
        package: package.clone(),
        display: localized(spec.display_name, spec.description),
        requires: Vec::new(),
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
        CHANNEL_REPLY => serde_json::json!({
            "type": "object",
            "properties": {
                "destination_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "message_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "text": { "type": "string", "minLength": 1, "maxLength": 16384 }
            },
            "required": ["destination_ref", "message_ref", "text"],
            "additionalProperties": false
        }),
        CHANNEL_SEND => serde_json::json!({
            "type": "object",
            "properties": {
                "destination_ref": { "type": "string", "minLength": 1, "maxLength": 512 },
                "text": { "type": "string", "minLength": 1, "maxLength": 16384 }
            },
            "required": ["destination_ref", "text"],
            "additionalProperties": false
        }),
        COMPANION_LEARN | COMPANION_EVOLVE => serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "minLength": 1, "maxLength": 512 }
            },
            "additionalProperties": false
        }),
        ROBOT_DISPLAY | ROBOT_MOTION | ROBOT_DEVICE_TOOLS => serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": { "type": "string", "minLength": 1, "maxLength": 512 },
                "arguments": { "type": "object" }
            },
            "required": ["tool_name", "arguments"],
            "additionalProperties": false
        }),
        CUSTOMER_SERVICE_NOTES_READ => serde_json::json!({
            "type": "object",
            "properties": {
                "cs_note_id": { "type": "string" },
                "include_disabled": { "type": "boolean", "default": false },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
            },
            "additionalProperties": false
        }),
        CUSTOMER_SERVICE_NOTES_WRITE => serde_json::json!({
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
        CUSTOMER_SERVICE_HANDOFF => serde_json::json!({
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
        ROBOT_VISION => serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "const": "robot_vision" },
                "robot_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "companion_id": { "type": "string", "minLength": 1, "maxLength": 512 },
                "question": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "answer": { "type": "string", "minLength": 1, "maxLength": 65536 },
                "observed_at_ms": { "type": "integer", "minimum": 0 }
            },
            "required": ["kind", "robot_id", "companion_id", "question", "answer", "observed_at_ms"],
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
    action_id: ActionId,
    requirements: &'static [ResourceRequirement],
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
            if context.capability_id != self.capability_id
                || context.action_id != self.action_id
            {
                return Err(KernelError::ActionNotDeclared {
                    capability_id: context.capability_id,
                    action_id: context.action_id,
                });
            }
            let operation = operation_from_input(&self.capability_id, input)?;

            validate_resource_bindings(
                &self.capability_id,
                &context.principal.principal_id,
                self.requirements,
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
                    action_id: self.action_id.clone(),
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
    let operation = match capability_id.as_ref() {
        CHANNEL_REPLY => Wave4CapabilityOperation::ChannelReply { input },
        CHANNEL_SEND => Wave4CapabilityOperation::ChannelSend { input },
        COMPANION_LEARN => Wave4CapabilityOperation::CompanionLearn { input },
        COMPANION_EVOLVE => Wave4CapabilityOperation::CompanionEvolve { input },
        CUSTOMER_SERVICE_NOTES_READ => {
            Wave4CapabilityOperation::CustomerServiceNotesRead { input }
        }
        CUSTOMER_SERVICE_NOTES_WRITE => {
            Wave4CapabilityOperation::CustomerServiceNotesWrite { input }
        }
        CUSTOMER_SERVICE_HANDOFF => Wave4CapabilityOperation::CustomerServiceHandoff { input },
        ROBOT_DISPLAY => Wave4CapabilityOperation::RobotDisplay { input },
        ROBOT_MOTION => Wave4CapabilityOperation::RobotMotion { input },
        ROBOT_DEVICE_TOOLS => Wave4CapabilityOperation::RobotDeviceTools { input },
        other => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{other} does not expose an action host operation"),
            });
        }
    };
    if !operation.input().0.is_object() {
        return Err(wave4_host_error_to_kernel(Wave4HostPortError::invalid_request(
            format!("{} input must be a JSON object", capability_id.as_ref()),
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
mod tests {
    use std::sync::Mutex;
    use std::task::{Context, Poll, Wake, Waker};

    use super::*;
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
    };

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test host owner must settle immediately"),
        }
    }

    fn valid_context(
        capability_id: &str,
        action_id: &str,
        resource_kind: &str,
    ) -> Wave4HostContext {
        let owner_id = "wave4-test-owner";
        let resource_bindings = canonical_resource_bindings(owner_id)
            .into_iter()
            .filter(|binding| binding.resource_kind.as_ref() == resource_kind)
            .collect();
        Wave4HostContext {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: owner_id.to_owned(),
            },
            agent_session_id: AgentSessionId::from("wave4-test-session"),
            operation_id: OperationId::from("wave4-test-operation"),
            idempotency_key: IdempotencyKey::from("wave4-test-idempotency"),
            correlation_id: CorrelationId::from("wave4-test-correlation"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "digest".into(),
            },
            registry_generation: 1,
            capability_id: CapabilityId::from(capability_id),
            action_id: ActionId::from(action_id),
            state_scope_key: ScopeKey::from("session:wave4-test"),
            resource_bindings,
        }
    }

    fn valid_request(
        capability_id: &str,
        action_id: &str,
        resource_kind: &str,
        operation: Wave4CapabilityOperation,
    ) -> Wave4HostRequest {
        Wave4HostRequest {
            context: valid_context(capability_id, action_id, resource_kind),
            operation,
        }
    }

    fn valid_context_request(
        capability_id: &str,
        resource_kind: &str,
    ) -> Wave4ContextHostRequest {
        let owner_id = "wave4-test-owner";
        Wave4ContextHostRequest {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: owner_id.to_owned(),
            },
            agent_session_id: AgentSessionId::from("wave4-test-session"),
            operation_id: OperationId::from("wave4-context-operation"),
            correlation_id: CorrelationId::from("wave4-context-correlation"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "digest".into(),
            },
            registry_generation: 1,
            registry_digest: DigestHex::from("registry-digest"),
            capability_id: CapabilityId::from(capability_id),
            state_scope_key: ScopeKey::from("session:wave4-test"),
            resource_bindings: canonical_resource_bindings(owner_id)
                .into_iter()
                .filter(|binding| binding.resource_kind.as_ref() == resource_kind)
                .collect(),
            schema_ref: schema_ref(
                capability_id,
                "context",
                &context_output_schema(capability_id),
            )
                .expect("canonical Context schema"),
        }
    }

    fn object_with_message() -> StrictJsonValue {
        let mut input = empty_object();
        input
            .0
            .as_object_mut()
            .expect("empty object")
            .insert("message".to_owned(), "hello".into());
        input
    }

    #[test]
    fn registrations_cover_the_full_target_package_inventory() {
        let registrations = registrations().expect("Wave 4 registrations should build");
        assert_eq!(registrations.len(), PACKAGE_IDS.len());

        let expected = BTreeMap::from([
            (
                CHANNEL_PACKAGE_ID.to_owned(),
                BTreeSet::from([
                    CHANNEL_RECEIVE.to_owned(),
                    CHANNEL_REPLY.to_owned(),
                    CHANNEL_SEND.to_owned(),
                    CHANNEL_PAIRING.to_owned(),
                    CHANNEL_GROUP_POLICY.to_owned(),
                ]),
            ),
            (
                COMPANION_PACKAGE_ID.to_owned(),
                BTreeSet::from([
                    COMPANION_EVOLVE.to_owned(),
                    COMPANION_LEARN.to_owned(),
                    COMPANION_PERSONA.to_owned(),
                    COMPANION_ROSTER.to_owned(),
                ]),
            ),
            (
                CUSTOMER_SERVICE_PACKAGE_ID.to_owned(),
                BTreeSet::from([
                    CUSTOMER_SERVICE_DIALOGUE.to_owned(),
                    CUSTOMER_SERVICE_NOTES_READ.to_owned(),
                    CUSTOMER_SERVICE_NOTES_WRITE.to_owned(),
                    CUSTOMER_SERVICE_HANDOFF.to_owned(),
                ]),
            ),
            (
                ROBOT_PACKAGE_ID.to_owned(),
                BTreeSet::from([
                    ROBOT_LINK.to_owned(),
                    ROBOT_AUDIO.to_owned(),
                    ROBOT_DEVICE_TOOLS.to_owned(),
                    ROBOT_DISPLAY.to_owned(),
                    ROBOT_MOTION.to_owned(),
                    ROBOT_VISION.to_owned(),
                ]),
            ),
            (
                NOTIFICATION_PACKAGE_ID.to_owned(),
                BTreeSet::from([
                    NOTIFICATION_WEBHOOK.to_owned(),
                    NOTIFICATION_DESKTOP.to_owned(),
                ]),
            ),
        ]);

        let mut observed = BTreeMap::new();
        let expected_kinds = BTreeMap::from([
            (CHANNEL_RECEIVE, CapabilityKind::EventSource),
            (CHANNEL_REPLY, CapabilityKind::Tool),
            (CHANNEL_SEND, CapabilityKind::Tool),
            (CHANNEL_PAIRING, CapabilityKind::Transport),
            (CHANNEL_GROUP_POLICY, CapabilityKind::TurnMiddleware),
            (COMPANION_PERSONA, CapabilityKind::ContextContributor),
            (COMPANION_ROSTER, CapabilityKind::ContextContributor),
            (COMPANION_LEARN, CapabilityKind::Tool),
            (COMPANION_EVOLVE, CapabilityKind::Tool),
            (CUSTOMER_SERVICE_DIALOGUE, CapabilityKind::TurnMiddleware),
            (CUSTOMER_SERVICE_NOTES_READ, CapabilityKind::Tool),
            (CUSTOMER_SERVICE_NOTES_WRITE, CapabilityKind::Tool),
            (CUSTOMER_SERVICE_HANDOFF, CapabilityKind::Tool),
            (ROBOT_LINK, CapabilityKind::ResourceProvider),
            (ROBOT_AUDIO, CapabilityKind::BackgroundService),
            (ROBOT_VISION, CapabilityKind::ContextContributor),
            (ROBOT_DISPLAY, CapabilityKind::Tool),
            (ROBOT_MOTION, CapabilityKind::Tool),
            (ROBOT_DEVICE_TOOLS, CapabilityKind::Tool),
            (NOTIFICATION_WEBHOOK, CapabilityKind::EventConsumer),
            (NOTIFICATION_DESKTOP, CapabilityKind::EventConsumer),
        ]);
        for registration in registrations {
            let manifest = &registration.metadata.manifest.payload;
            assert_eq!(manifest.package_version.as_ref(), PACKAGE_VERSION);
            assert_eq!(
                registration.metadata.source.source_kind,
                PluginSourceKind::Bundled
            );
            assert_eq!(
                registration.metadata.source.source_identity,
                manifest.package_id.as_ref()
            );
            assert_eq!(
                manifest
                    .entrypoint
                    .as_in_process()
                    .expect("first-party entrypoint must be in-process")
                    .entrypoint_profile,
                "trusted-in-process"
            );
            let ids = manifest
                .contributions
                .capabilities
                .iter()
                .map(|capability| capability.id.as_ref().to_owned())
                .collect::<BTreeSet<_>>();
            observed.insert(manifest.package_id.as_ref().to_owned(), ids);

            for capability in &manifest.contributions.capabilities {
                assert_eq!(
                    capability.kind,
                    expected_kinds[capability.id.as_ref()]
                );
                if capability.kind == CapabilityKind::Tool {
                    assert_eq!(capability.contributions.actions.len(), 1);
                    assert_eq!(
                        capability.contributions.actions[0].action_id,
                        action_id_for(capability.id.as_ref())
                    );
                    assert!(registration.handler_ids().contains(&capability.id));
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(!registration.handler_ids().contains(&capability.id));
                }
            }
        }
        assert_eq!(observed, expected);

        let all = observed
            .values()
            .flat_map(|capabilities| capabilities.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            all,
            TARGET_CAPABILITY_IDS
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<BTreeSet<_>>()
        );
        assert!(all.contains(CHANNEL_PAIRING));
        assert!(all.contains(CHANNEL_GROUP_POLICY));
        assert!(all.contains(ROBOT_LINK));
        assert!(all.contains(NOTIFICATION_DESKTOP));
        assert!(all.contains(CUSTOMER_SERVICE_NOTES_READ));
        assert!(all.contains(CUSTOMER_SERVICE_NOTES_WRITE));
    }

    #[test]
    fn typed_resource_descriptors_and_bindings_match_capability_metadata() {
        let descriptors = typed_resource_descriptors();
        let descriptor_kinds = descriptors
            .iter()
            .map(|descriptor| descriptor.resource_kind.as_ref())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            descriptor_kinds,
            BTreeSet::from([
                CHANNEL_RESOURCE_KIND,
                COMPANION_RESOURCE_KIND,
                COMPANION_MEMORY_RESOURCE_KIND,
                CUSTOMER_RESOURCE_KIND,
                ROBOT_RESOURCE_KIND,
            ])
        );

        let resource_metadata = resource_binding_metadata();
        let required_kinds = resource_metadata
            .keys()
            .map(AsRef::as_ref)
            .collect::<BTreeSet<_>>();
        assert!(required_kinds.contains(CHANNEL_RESOURCE_KIND));
        assert!(required_kinds.contains(COMPANION_RESOURCE_KIND));
        assert!(required_kinds.contains(COMPANION_MEMORY_RESOURCE_KIND));
        assert!(required_kinds.contains(CUSTOMER_RESOURCE_KIND));
        assert!(required_kinds.contains(ROBOT_RESOURCE_KIND));
        assert_eq!(
            resource_metadata[&ResourceKind::from(CHANNEL_RESOURCE_KIND)],
            BTreeSet::from([
                "manage".to_owned(),
                "receive".to_owned(),
                "reply".to_owned(),
                "send".to_owned(),
            ])
        );
        assert_eq!(
            resource_metadata[&ResourceKind::from(COMPANION_RESOURCE_KIND)],
            BTreeSet::from(["read".to_owned(), "write".to_owned()])
        );
        assert_eq!(
            resource_metadata[&ResourceKind::from(COMPANION_MEMORY_RESOURCE_KIND)],
            BTreeSet::from(["read".to_owned(), "write".to_owned()])
        );
        assert_eq!(
            resource_metadata[&ResourceKind::from(CUSTOMER_RESOURCE_KIND)],
            BTreeSet::from(["read".to_owned(), "write".to_owned()])
        );
        assert_eq!(
            resource_metadata[&ResourceKind::from(ROBOT_RESOURCE_KIND)],
            BTreeSet::from([
                "audio".to_owned(),
                "display".to_owned(),
                "link".to_owned(),
                "motion".to_owned(),
                "vision".to_owned(),
            ])
        );

        assert_eq!(
            required_resource_kinds(CHANNEL_REPLY),
            Some(BTreeSet::from([ResourceKind::from(CHANNEL_RESOURCE_KIND)]))
        );
        assert_eq!(
            required_resource_kinds(COMPANION_LEARN),
            Some(BTreeSet::from([ResourceKind::from(
                COMPANION_MEMORY_RESOURCE_KIND
            )]))
        );
        assert_eq!(
            required_resource_kinds(ROBOT_MOTION),
            Some(BTreeSet::from([ResourceKind::from(ROBOT_RESOURCE_KIND)]))
        );

        let bindings = canonical_resource_bindings("owner-1");
        assert_eq!(bindings.len(), descriptors.len());
        assert!(bindings.iter().all(|binding| binding.owner_id == "owner-1"));
        assert!(bindings.iter().all(|binding| {
            descriptors
                .iter()
                .any(|descriptor| {
                    descriptor.resource_kind == binding.resource_kind
                        && descriptor.operations == binding.operations
                })
        }));
        assert!(bindings.iter().all(|binding| {
            binding.connection_config_ref.is_none() && binding.typed_parameters.is_empty()
        }));

        let custom = typed_resource_binding(
            "customer-binding",
            CUSTOMER_RESOURCE_KIND,
            "customer-1",
            "owner-1",
            ["read", "write"],
        );
        assert_eq!(custom.binding_id.as_ref(), "customer-binding");
        assert_eq!(custom.resource_kind.as_ref(), CUSTOMER_RESOURCE_KIND);
        assert_eq!(custom.resource_id.as_ref(), "customer-1");
        assert_eq!(custom.owner_id, "owner-1");
        assert_eq!(
            custom.operations,
            BTreeSet::from(["read".to_owned(), "write".to_owned()])
        );
        assert!(custom.connection_config_ref.is_none());
        assert!(custom.typed_parameters.is_empty());
    }

    #[test]
    fn handler_resource_requirements_fit_the_frozen_binding_operations() {
        for capability in all_capabilities() {
            let bindings = canonical_resource_bindings("owner-1")
                .into_iter()
                .filter(|binding| {
                    capability
                        .resource_kinds
                        .contains(&binding.resource_kind.as_ref())
                })
                .collect::<Vec<_>>();
            for requirement in capability.requirements {
                let binding = bindings
                    .iter()
                    .find(|binding| binding.resource_kind.as_ref() == requirement.resource_kind)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} requires missing resource kind {}",
                            capability.id, requirement.resource_kind
                        )
                    });
                assert!(
                    binding.operations.contains(requirement.operation),
                    "{} requires operation {} on {} but the frozen binding exposes {:?}",
                    capability.id,
                    requirement.operation,
                    requirement.resource_kind,
                    binding.operations
                );
            }
        }
    }

    #[test]
    fn every_action_capability_has_an_exact_typed_operation_and_action_pair() {
        for capability in all_capabilities().filter(|capability| capability.effect_class.is_some()) {
            let capability_id = CapabilityId::from(capability.id);
            let operation = operation_from_input(&capability_id, empty_object())
                .expect("every action capability must have a typed operation");
            assert_eq!(operation.capability_id(), capability_id);
            assert_eq!(operation.action_id(), canonical_action_id(capability.id).unwrap());
            assert_eq!(
                action_id_for(capability.id),
                ActionId::from(format!("{}.invoke", capability.id))
            );
        }
        for capability in all_capabilities().filter(|capability| capability.effect_class.is_none()) {
            assert!(canonical_action_id(capability.id).is_none());
            assert!(operation_from_input(&CapabilityId::from(capability.id), empty_object()).is_err());
        }
    }

    #[test]
    fn canonical_action_matrix_maps_each_capability_to_its_exact_variant_and_owner() {
        let cases = [
            (CHANNEL_REPLY, CHANNEL_REPLY_ACTION, Wave4OwnerDomain::Channel),
            (CHANNEL_SEND, CHANNEL_SEND_ACTION, Wave4OwnerDomain::Channel),
            (
                COMPANION_LEARN,
                COMPANION_LEARN_ACTION,
                Wave4OwnerDomain::Companion,
            ),
            (
                COMPANION_EVOLVE,
                COMPANION_EVOLVE_ACTION,
                Wave4OwnerDomain::Companion,
            ),
            (
                CUSTOMER_SERVICE_NOTES_READ,
                CUSTOMER_SERVICE_NOTES_READ_ACTION,
                Wave4OwnerDomain::CustomerService,
            ),
            (
                CUSTOMER_SERVICE_NOTES_WRITE,
                CUSTOMER_SERVICE_NOTES_WRITE_ACTION,
                Wave4OwnerDomain::CustomerService,
            ),
            (
                CUSTOMER_SERVICE_HANDOFF,
                CUSTOMER_SERVICE_HANDOFF_ACTION,
                Wave4OwnerDomain::CustomerService,
            ),
            (
                ROBOT_DISPLAY,
                ROBOT_DISPLAY_ACTION,
                Wave4OwnerDomain::Robot,
            ),
            (ROBOT_MOTION, ROBOT_MOTION_ACTION, Wave4OwnerDomain::Robot),
            (
                ROBOT_DEVICE_TOOLS,
                ROBOT_DEVICE_TOOLS_ACTION,
                Wave4OwnerDomain::Robot,
            ),
        ];

        for (capability_id, action_id, owner_domain) in cases {
            let input = object_with_message();
            let operation =
                operation_from_input(&CapabilityId::from(capability_id), input.clone())
                    .expect("canonical action capability");
            assert_eq!(operation.input(), &input);
            assert_eq!(operation.capability_id().as_ref(), capability_id);
            assert_eq!(operation.action_id().as_ref(), action_id);
            assert_eq!(operation.owner_domain(), owner_domain);
            match capability_id {
                CHANNEL_REPLY => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::ChannelReply { .. }
                )),
                CHANNEL_SEND => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::ChannelSend { .. }
                )),
                COMPANION_LEARN => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CompanionLearn { .. }
                )),
                COMPANION_EVOLVE => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CompanionEvolve { .. }
                )),
                CUSTOMER_SERVICE_NOTES_READ => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CustomerServiceNotesRead { .. }
                )),
                CUSTOMER_SERVICE_NOTES_WRITE => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CustomerServiceNotesWrite { .. }
                )),
                CUSTOMER_SERVICE_HANDOFF => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CustomerServiceHandoff { .. }
                )),
                ROBOT_DISPLAY => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::RobotDisplay { .. }
                )),
                ROBOT_MOTION => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::RobotMotion { .. }
                )),
                ROBOT_DEVICE_TOOLS => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::RobotDeviceTools { .. }
                )),
                _ => unreachable!("all canonical action cases are explicit"),
            }
        }
    }

    #[test]
    fn host_request_validation_accepts_each_action_with_its_declared_resource_operation() {
        for capability in all_capabilities().filter(|capability| capability.effect_class.is_some()) {
            assert_eq!(
                capability.requirements.len(),
                1,
                "{} must have one canonical resource requirement",
                capability.id
            );
            let requirement = capability.requirements[0];
            let capability_id = CapabilityId::from(capability.id);
            let action_id = canonical_action_id(capability.id).expect("action identity");
            let operation =
                operation_from_input(&capability_id, empty_object()).expect("typed operation");
            let request = valid_request(
                capability.id,
                action_id.as_ref(),
                requirement.resource_kind,
                operation,
            );
            request
                .validate()
                .unwrap_or_else(|error| panic!("{} must validate: {error}", capability.id));
        }
    }

    #[test]
    fn host_request_validation_rejects_cross_capability_operations_and_transport_branches() {
        let mismatched = valid_request(
            CHANNEL_SEND,
            CHANNEL_SEND_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        );
        let error = mismatched.validate().expect_err("cross-capability operation must reject");
        assert_eq!(error.code, WAVE4_ACTION_OPERATION_MISMATCH);

        let pairing = Wave4HostRequest {
            context: valid_context(CHANNEL_PAIRING, "channel.pairing.invoke", CHANNEL_RESOURCE_KIND),
            operation: Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        };
        let error = pairing
            .validate()
            .expect_err("pairing must remain transport-owned");
        assert_eq!(error.code, WAVE4_ACTION_OPERATION_MISMATCH);
    }

    #[test]
    fn host_request_validation_rejects_invalid_binding_metadata_before_owner_dispatch() {
        let mut request = valid_request(
            CHANNEL_REPLY,
            CHANNEL_REPLY_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        );
        request.context.resource_bindings[0].owner_id = "another-owner".to_owned();
        let error = request
            .validate()
            .expect_err("foreign resource owner must reject");
        assert_eq!(error.code, WAVE4_RESOURCE_OWNER_MISMATCH);

        request.context.resource_bindings[0].owner_id = "wave4-test-owner".to_owned();
        request.context.resource_bindings[0]
            .operations
            .insert("not-declared".to_owned());
        let error = request
            .validate()
            .expect_err("undeclared resource operation must reject");
        assert_eq!(error.code, WAVE4_RESOURCE_BINDING_INVALID);

        request.context.resource_bindings.push(typed_resource_binding(
            "wave4-robot",
            ROBOT_RESOURCE_KIND,
            "robot-1",
            "wave4-test-owner",
            ["link", "display", "motion", "audio", "vision"],
        ));
        let error = request
            .validate()
            .expect_err("unexpected resource kind must reject");
        assert_eq!(error.code, WAVE4_RESOURCE_BINDING_INVALID);

        let mut missing = valid_request(
            CHANNEL_REPLY,
            CHANNEL_REPLY_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        );
        missing.context.resource_bindings.clear();
        let error = missing
            .validate()
            .expect_err("an unbound target resource must remain configurable");
        assert_eq!(error.code, WAVE4_RESOURCE_NOT_BOUND);
        assert_ne!(error.code, WAVE4_HOST_PORT_UNAVAILABLE);

        let read_only = typed_resource_binding(
            "wave4-channel",
            CHANNEL_RESOURCE_KIND,
            "channel-1",
            "wave4-test-owner",
            ["receive"],
        );
        missing.context.resource_bindings = vec![read_only];
        let error = missing
            .validate()
            .expect_err("a binding without the requested operation must be configurable");
        assert_eq!(error.code, WAVE4_RESOURCE_NOT_BOUND);
    }

    #[test]
    fn kernel_binding_validation_preserves_the_canonical_unbound_resource_error() {
        let capability_id = CapabilityId::from(CHANNEL_REPLY);
        let requirements = find_capability(CHANNEL_REPLY)
            .expect("channel.reply capability")
            .requirements;
        let error = validate_resource_bindings(
            &capability_id,
            "wave4-test-owner",
            requirements,
            &[],
        )
        .expect_err("missing target binding must reject before owner dispatch");
        assert!(matches!(
            error,
            KernelError::ResourceBindingMissing { ref binding_id }
                if binding_id.as_ref() == CHANNEL_RESOURCE_KIND
        ));
        assert_eq!(error.canonical_code().as_ref(), WAVE4_RESOURCE_NOT_BOUND);
    }

    #[test]
    fn context_host_request_preserves_binding_and_schema_authority() {
        let request = valid_context_request(
            COMPANION_PERSONA,
            COMPANION_RESOURCE_KIND,
        );
        request.validate().expect("valid companion Context request");

        let mut missing = request.clone();
        missing.resource_bindings.clear();
        assert_eq!(
            missing.validate().unwrap_err().code,
            WAVE4_RESOURCE_NOT_BOUND
        );

        let mut wrong_owner = request.clone();
        wrong_owner.resource_bindings[0].owner_id = "other-owner".to_owned();
        assert_eq!(
            wrong_owner.validate().unwrap_err().code,
            WAVE4_RESOURCE_OWNER_MISMATCH
        );

        let mut wrong_schema = request;
        wrong_schema.schema_ref = CanonicalSchemaRef::from("schema://wrong");
        assert_eq!(
            wrong_schema.validate().unwrap_err().code,
            WAVE4_INVALID_REQUEST
        );
    }

    #[test]
    fn composed_context_host_routes_only_to_the_matching_real_owner() {
        struct ContextOwner {
            calls: Arc<Mutex<Vec<String>>>,
        }

        impl Wave4ContextHostPort for ContextOwner {
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
                let calls = Arc::clone(&self.calls);
                Box::pin(async move {
                    request.validate()?;
                    calls
                        .lock()
                        .unwrap()
                        .push(request.capability_id.as_ref().to_owned());
                    let mut value = empty_object();
                    value
                        .0
                        .as_object_mut()
                        .expect("empty object")
                        .insert("owner".to_owned(), "companion".into());
                    Ok(Some(value))
                })
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let host = composed_context_host_port(
            Wave4ContextOwnerBindings::default().with_companion(Arc::new(
                ContextOwner {
                    calls: Arc::clone(&calls),
                },
            )),
        );
        let result = poll_ready(host.contribute(valid_context_request(
            COMPANION_PERSONA,
            COMPANION_RESOURCE_KIND,
        )))
        .expect("companion Context owner must run");
        assert_eq!(result.unwrap().0["owner"], "companion");
        assert_eq!(calls.lock().unwrap().as_slice(), [COMPANION_PERSONA]);

        let error = poll_ready(host.contribute(valid_context_request(
            ROBOT_VISION,
            ROBOT_RESOURCE_KIND,
        )))
        .expect_err("missing robot Context owner must fail closed");
        assert_eq!(error.code, WAVE4_HOST_PORT_UNAVAILABLE);
    }

    #[test]
    fn composed_host_port_routes_only_to_injected_owner_and_keeps_missing_owner_unavailable() {
        struct RejectingOwner {
            calls: Arc<Mutex<Vec<Wave4OwnerDomain>>>,
            domain: Wave4OwnerDomain,
        }

        impl Wave4HostPort for RejectingOwner {
            fn invoke<'a>(
                &'a self,
                request: Wave4HostRequest,
            ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>>
            {
                let calls = Arc::clone(&self.calls);
                let domain = self.domain;
                Box::pin(async move {
                    request.validate()?;
                    calls.lock().unwrap().push(domain);
                    Err(Wave4HostPortError::new(
                        "TEST_OWNER_REJECTED",
                        "the boundary test owner never projects success",
                    ))
                })
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let host = composed_host_port(
            Wave4OwnerBindings::default().with_channel(Arc::new(RejectingOwner {
                calls: Arc::clone(&calls),
                domain: Wave4OwnerDomain::Channel,
            })),
        );
        let request = valid_request(
            CHANNEL_REPLY,
            CHANNEL_REPLY_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: object_with_message(),
            },
        );
        let error =
            poll_ready(host.invoke(request)).expect_err("boundary owner deliberately rejects");
        assert_eq!(error.code, "TEST_OWNER_REJECTED");
        assert_eq!(*calls.lock().unwrap(), vec![Wave4OwnerDomain::Channel]);

        let kernel_error = wave4_host_error_to_kernel(error);
        let failure = kernel_error
            .capability_execution_failure()
            .expect("Wave 4 host errors cross the Kernel as a typed failure");
        assert_eq!(failure.code.as_ref(), "TEST_OWNER_REJECTED");
        assert_eq!(
            failure.message,
            "the boundary test owner never projects success"
        );

        let invalid = valid_request(
            CHANNEL_SEND,
            CHANNEL_SEND_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        );
        let error = poll_ready(host.invoke(invalid)).expect_err("invalid request must reject");
        assert_eq!(error.code, WAVE4_ACTION_OPERATION_MISMATCH);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![Wave4OwnerDomain::Channel],
            "invalid requests must not reach an injected owner"
        );

        let missing_owner = composed_host_port(Wave4OwnerBindings::default());
        let error = poll_ready(missing_owner.invoke(valid_request(
            ROBOT_DISPLAY,
            ROBOT_DISPLAY_ACTION,
            ROBOT_RESOURCE_KIND,
            Wave4CapabilityOperation::RobotDisplay {
                input: empty_object(),
            },
        )))
        .expect_err("missing Robot owner must remain unavailable");
        assert_eq!(error.code, WAVE4_HOST_PORT_UNAVAILABLE);
    }

    #[test]
    fn channel_and_robot_availability_stays_on_host_execution_surfaces() {
        let expected_surfaces = capability_surface_declarations(
            AGENT_SURFACES.iter().copied(),
            [CapabilityConsumer::Agent],
        );
        for registration in [
            channel_registration().expect("channel registration"),
            robot_registration().expect("robot registration"),
        ] {
            for capability in &registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
            {
                assert_eq!(capability.supported_surfaces, expected_surfaces);
                assert_eq!(
                    capability.supported_platforms,
                    vec![PlatformConstraint::Any]
                );
                for remote_only_client in [
                    "im-client",
                    "mobile",
                    "robot-firmware",
                    "web-browser-client",
                ] {
                    assert!(!capability.supported_surfaces.contains(remote_only_client));
                }
            }
        }
    }

    #[test]
    fn registrations_materialize_without_legacy_surface_or_partial_publish() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence should initialize");
        let materialized = registry
            .replace_all(registrations().expect("registrations should build"))
            .expect("Wave 4 registrations should materialize");

        assert_eq!(materialized.packages.len(), PACKAGE_IDS.len());
        assert_eq!(materialized.capabilities.len(), TARGET_CAPABILITY_IDS.len());
        assert_eq!(materialized.generation, 1);
        assert!(materialized
            .capabilities
            .contains_key(&CapabilityId::from(CHANNEL_RECEIVE)));
        assert!(materialized
            .capabilities
            .contains_key(&CapabilityId::from(ROBOT_VISION)));
    }

    #[test]
    fn ingress_ports_are_typed_and_match_each_registration_declaration() {
        let registrations = registrations().expect("registrations should build");
        for registration in &registrations {
            let context = &registration.metadata.context;
            let declared = context
                .host_ports
                .iter()
                .map(|port| port.port.id.clone())
                .chain(
                    context
                        .typed_command_ports
                        .iter()
                        .map(|port| port.port.id.clone()),
                )
                .chain(
                    context
                        .domain_outbox_ports
                        .iter()
                        .map(|port| port.port.id.clone()),
                )
                .chain([context.cancellation.cancellation_port.id.clone()])
                .chain([context.managed_task_registration.registrar_port.id.clone()])
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.metadata.registrar.declared_host_ports, declared);
            assert!(context
                .typed_command_ports
                .iter()
                .all(|port| port.command_schema.as_ref().starts_with("schema://")));
            assert!(context
                .typed_command_ports
                .iter()
                .all(|port| port.receipt_schema.as_ref().starts_with("schema://")));
            assert!(context
                .domain_outbox_ports
                .iter()
                .all(|port| port.event_schema.as_ref().starts_with("schema://")));
            assert!(context
                .domain_outbox_ports
                .iter()
                .all(|port| port.cursor_schema.as_ref().starts_with("schema://")));
        }

        let channel = channel_registration().expect("channel registration");
        assert_eq!(
            channel
                .metadata
                .context
                .typed_command_ports
                .iter()
                .map(|port| port.port.id.as_ref())
                .collect::<Vec<_>>(),
            vec!["channel.agent-session-command", "channel.inbound-receipt"]
        );
        let notification = notification_registration().expect("notification registration");
        assert_eq!(
            notification
                .metadata
                .context
                .domain_outbox_ports
                .iter()
                .map(|port| port.port.id.as_ref())
                .collect::<Vec<_>>(),
            vec!["notification.webhook-outbox"]
        );
    }

    #[test]
    fn family_normalization_keeps_inventory_ids_canonical() {
        assert_eq!(
            canonical_capability_id("customer-service.dialogue"),
            Some(CapabilityId::from(CUSTOMER_SERVICE_DIALOGUE))
        );
        assert_eq!(
            canonical_capability_id("robot.device-tools"),
            Some(CapabilityId::from(ROBOT_DEVICE_TOOLS))
        );
        assert_eq!(
            canonical_capability_id(CHANNEL_PAIRING),
            Some(CapabilityId::from(CHANNEL_PAIRING))
        );
        assert_eq!(
            canonical_capability_id(ROBOT_LINK),
            Some(CapabilityId::from(ROBOT_LINK))
        );
    }

    #[test]
    fn action_capabilities_use_the_wave4_host_port_and_pairing_stays_transport_owned() {
        let registrations = registrations().expect("registrations should build");
        for registration in registrations {
            let context = &registration.metadata.context;
            let capabilities = &registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities;
            let action_capabilities = capabilities
                .iter()
                .filter(|capability| capability.kind == CapabilityKind::Tool)
                .collect::<Vec<_>>();
            let host_port_ids = context
                .host_ports
                .iter()
                .map(|binding| binding.port.id.as_ref())
                .collect::<BTreeSet<_>>();

            if action_capabilities.is_empty() {
                assert!(!host_port_ids.contains(WAVE4_CAPABILITY_HOST_PORT_ID));
            } else {
                assert!(host_port_ids.contains(WAVE4_CAPABILITY_HOST_PORT_ID));
                for capability in action_capabilities {
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE4_CAPABILITY_HOST_PORT_ID)]
                    );
                }
            }
        }

        let channel = channel_registration().expect("channel registration");
        let pairing = channel
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .iter()
            .find(|capability| capability.id.as_ref() == CHANNEL_PAIRING)
            .expect("pairing capability");
        assert_eq!(pairing.kind, CapabilityKind::Transport);
        assert!(pairing.contributions.actions.is_empty());
        assert_eq!(
            pairing.contributions.host_ports,
            vec![host_port(WAVE4_LIFECYCLE_HOST_PORT_ID)]
        );
        assert!(!channel
            .handler_ids()
            .contains(&CapabilityId::from(CHANNEL_PAIRING)));
    }

    #[test]
    fn context_capabilities_use_the_typed_context_host_and_export_exact_schemas() {
        assert_eq!(empty_object().0, serde_json::json!({}));
        for additional_properties in [false, true] {
            assert_eq!(
                object_schema(additional_properties).0,
                serde_json::json!({
                    "type": "object",
                    "additionalProperties": additional_properties,
                })
            );
        }
        for registration in registrations().expect("Wave 4 registrations") {
            for capability in &registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
            {
                for reference in capability
                    .contributions
                    .actions
                    .iter()
                    .flat_map(|action| [&action.input_schema, &action.output_schema])
                    .chain(&capability.contributions.context_schema_refs)
                    .chain(&capability.contributions.event_schema_refs)
                {
                    assert!(
                        resolve_capability_schema(reference)
                            .expect("Wave 4 schema resolution")
                            .is_some(),
                        "missing schema bytes for {} / {}",
                        capability.id.as_ref(),
                        reference.as_ref()
                    );
                }
                if capability.kind == CapabilityKind::ContextContributor {
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE4_CONTEXT_HOST_PORT_ID)]
                    );
                } else if capability.kind == CapabilityKind::TurnMiddleware {
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE4_TURN_MIDDLEWARE_HOST_PORT_ID)]
                    );
                } else if matches!(
                    capability.kind,
                    CapabilityKind::EventSource | CapabilityKind::Transport
                ) {
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE4_LIFECYCLE_HOST_PORT_ID)]
                    );
                }
            }
        }
        assert!(
            resolve_capability_schema(&CanonicalSchemaRef::from("schema://unknown"))
                .unwrap()
                .is_none()
        );

        for capability_id in [
            CHANNEL_REPLY,
            CHANNEL_SEND,
            COMPANION_LEARN,
            COMPANION_EVOLVE,
            ROBOT_DISPLAY,
            ROBOT_MOTION,
            ROBOT_DEVICE_TOOLS,
            CUSTOMER_SERVICE_NOTES_READ,
            CUSTOMER_SERVICE_NOTES_WRITE,
            CUSTOMER_SERVICE_HANDOFF,
        ] {
            let input = action_input_schema(capability_id);
            assert_eq!(input.0["additionalProperties"], false);
            assert_eq!(input.0["type"], "object");
            let reference = schema_ref(capability_id, "input", &input).unwrap();
            assert_eq!(
                resolve_capability_schema(&reference).unwrap(),
                Some(input)
            );
        }
    }

    #[test]
    fn action_decoder_rejects_non_object_input_with_typed_invalid_request() {
        for capability in all_capabilities().filter(|capability| capability.effect_class.is_some()) {
            for input in [
                serde_json::json!(null),
                serde_json::json!(true),
                serde_json::json!(42),
                serde_json::json!("text"),
                serde_json::json!([]),
            ] {
                let error = operation_from_input(
                    &CapabilityId::from(capability.id),
                    StrictJsonValue(input),
                )
                .expect_err("every action requires an object payload before owner dispatch");
                assert_eq!(error.canonical_code().as_ref(), WAVE4_INVALID_REQUEST);
            }
        }
    }

    #[test]
    fn unconfigured_host_port_fails_closed_without_a_success_projection() {
        let host_port = unconfigured_host_port();
        let result = poll_ready(host_port.invoke(valid_request(
            CHANNEL_REPLY,
            CHANNEL_REPLY_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: empty_object(),
            },
        )));
        let error = result.expect_err("unconfigured host port must reject the action");
        assert_eq!(error.code, "WAVE4_HOST_PORT_UNAVAILABLE");
    }
}
