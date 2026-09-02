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
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
    CanonicalSchemaRef, CancellationDescriptor, CorrelationId,
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
    TypedResourceBindings, ValidatedPluginConfig, VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
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
pub const COMPANION_SUMMON: &str = "companion.summon";
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

pub const COMPANION_SUMMON_ACTION: &str = "companion.summon.invoke";
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
pub const TARGET_CAPABILITY_IDS: [&str; 22] = [
    CHANNEL_RECEIVE,
    CHANNEL_REPLY,
    CHANNEL_SEND,
    CHANNEL_PAIRING,
    CHANNEL_GROUP_POLICY,
    COMPANION_PERSONA,
    COMPANION_ROSTER,
    COMPANION_SUMMON,
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
pub const CAPABILITY_IDS: [&str; 22] = TARGET_CAPABILITY_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 22] = TARGET_CAPABILITY_IDS;

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
pub const WAVE4_HOST_PORT_UNAVAILABLE: &str = "WAVE4_HOST_PORT_UNAVAILABLE";
pub const WAVE4_INVALID_REQUEST: &str = "WAVE4_INVALID_REQUEST";
pub const WAVE4_ACTION_OPERATION_MISMATCH: &str = "WAVE4_ACTION_OPERATION_MISMATCH";
pub const WAVE4_RESOURCE_BINDING_INVALID: &str = "WAVE4_RESOURCE_BINDING_INVALID";
pub const WAVE4_RESOURCE_OWNER_MISMATCH: &str = "RESOURCE_OWNER_MISMATCH";
pub const WAVE4_EFFECT_CONTRACT_INVALID: &str = "WAVE4_EFFECT_CONTRACT_INVALID";
pub const WAVE4_EFFECT_TRANSITION_INVALID: &str = "WAVE4_EFFECT_TRANSITION_INVALID";

pub const CHANNEL_EFFECT_COMMAND_PORT_ID: &str = "channel.effect-command";
pub const CHANNEL_EFFECT_OUTBOX_PORT_ID: &str = "channel.effect-outbox";
pub const COMPANION_EFFECT_COMMAND_PORT_ID: &str = "companion.effect-command";
pub const COMPANION_EFFECT_OUTBOX_PORT_ID: &str = "companion.effect-outbox";
pub const CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID: &str =
    "customer-service.effect-command";
pub const CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID: &str =
    "customer-service.effect-outbox";
pub const ROBOT_EFFECT_COMMAND_PORT_ID: &str = "robot.effect-command";
pub const ROBOT_EFFECT_OUTBOX_PORT_ID: &str = "robot.effect-outbox";

const MAX_REFERENCE_CHARS: usize = 512;
const MAX_SHORT_TEXT_CHARS: usize = 512;
const MAX_MESSAGE_CHARS: usize = 16_384;
const MAX_CONTENT_CHARS: usize = 65_536;
const MAX_EFFECT_ERROR_CODE_CHARS: usize = 128;
const MAX_EFFECT_ERROR_MESSAGE_CHARS: usize = 2_048;
const MAX_EFFECT_RECEIPT_BYTES: usize = 65_536;
const MAX_OUTBOX_CURSOR_CHARS: usize = 512;
const MAX_DEVICE_RESULT_BYTES: usize = 16_384;
const MAX_NOTE_RESULTS: usize = 32;
const MAX_NOTE_CONTENT_CHARS: usize = 4_096;
const MAX_MEMORY_REFS: usize = 64;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelReplyRequest {
    pub message_ref: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSendRequest {
    pub destination_ref: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionSummonRequest {
    pub memory_refs: Vec<String>,
    pub purpose: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionLearnRequest {
    pub content: String,
    pub source_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionEvolveRequest {
    pub reason: String,
    pub expected_revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceNotesReadRequest {
    pub note_ref: Option<String>,
    pub query: Option<String>,
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomerServiceNoteWriteTarget {
    Create,
    Update {
        note_ref: String,
        expected_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceNotesWriteRequest {
    pub target: CustomerServiceNoteWriteTarget,
    pub content: String,
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceHandoffRequest {
    pub dialogue_ref: String,
    pub destination_ref: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotDisplayRequest {
    pub text: String,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RobotMotionRequest {
    pub motion: String,
    pub duration_ms: Option<u64>,
    pub parameters: Option<StrictJsonValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RobotDeviceToolsRequest {
    pub tool_name: String,
    pub arguments: StrictJsonValue,
}

/// Closed request vocabulary delivered to a production Wave 4 owner.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave4ActionRequest {
    ChannelReply(ChannelReplyRequest),
    ChannelSend(ChannelSendRequest),
    CompanionSummon(CompanionSummonRequest),
    CompanionLearn(CompanionLearnRequest),
    CompanionEvolve(CompanionEvolveRequest),
    CustomerServiceNotesRead(CustomerServiceNotesReadRequest),
    CustomerServiceNotesWrite(CustomerServiceNotesWriteRequest),
    CustomerServiceHandoff(CustomerServiceHandoffRequest),
    RobotDisplay(RobotDisplayRequest),
    RobotMotion(RobotMotionRequest),
    RobotDeviceTools(RobotDeviceToolsRequest),
}

impl Wave4ActionRequest {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::ChannelReply(_) => CHANNEL_REPLY,
            Self::ChannelSend(_) => CHANNEL_SEND,
            Self::CompanionSummon(_) => COMPANION_SUMMON,
            Self::CompanionLearn(_) => COMPANION_LEARN,
            Self::CompanionEvolve(_) => COMPANION_EVOLVE,
            Self::CustomerServiceNotesRead(_) => CUSTOMER_SERVICE_NOTES_READ,
            Self::CustomerServiceNotesWrite(_) => CUSTOMER_SERVICE_NOTES_WRITE,
            Self::CustomerServiceHandoff(_) => CUSTOMER_SERVICE_HANDOFF,
            Self::RobotDisplay(_) => ROBOT_DISPLAY,
            Self::RobotMotion(_) => ROBOT_MOTION,
            Self::RobotDeviceTools(_) => ROBOT_DEVICE_TOOLS,
        })
    }

    pub fn action_id(&self) -> ActionId {
        action_id_for(self.capability_id().as_ref())
    }

    pub fn owner_domain(&self) -> Wave4OwnerDomain {
        match self {
            Self::ChannelReply(_) | Self::ChannelSend(_) => Wave4OwnerDomain::Channel,
            Self::CompanionSummon(_)
            | Self::CompanionLearn(_)
            | Self::CompanionEvolve(_) => Wave4OwnerDomain::Companion,
            Self::CustomerServiceNotesRead(_)
            | Self::CustomerServiceNotesWrite(_)
            | Self::CustomerServiceHandoff(_) => Wave4OwnerDomain::CustomerService,
            Self::RobotDisplay(_) | Self::RobotMotion(_) | Self::RobotDeviceTools(_) => {
                Wave4OwnerDomain::Robot
            }
        }
    }

    fn validate(&self) -> Result<(), Wave4HostPortError> {
        let canonical = request_input_for_digest(self);
        let reparsed =
            parse_action_request(self.capability_id().as_ref(), &canonical)?;
        if reparsed != *self {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "typed Wave 4 request is not canonical",
            ));
        }
        Ok(())
    }
}

/// Kernel-facing ingress variants.
///
/// The raw JSON exists only until [`Wave4HostRequest::effect_command`] parses
/// it into [`Wave4ActionRequest`]. Production owners receive the latter.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave4CapabilityOperation {
    ChannelReply { input: StrictJsonValue },
    ChannelSend { input: StrictJsonValue },
    CompanionSummon { input: StrictJsonValue },
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
            Self::CompanionSummon { .. } => COMPANION_SUMMON,
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
            Self::CompanionSummon { .. }
            | Self::CompanionLearn { .. }
            | Self::CompanionEvolve { .. } => Wave4OwnerDomain::Companion,
            Self::CustomerServiceNotesRead { .. }
            | Self::CustomerServiceNotesWrite { .. }
            | Self::CustomerServiceHandoff { .. } => Wave4OwnerDomain::CustomerService,
            Self::RobotDisplay { .. }
            | Self::RobotMotion { .. }
            | Self::RobotDeviceTools { .. } => Wave4OwnerDomain::Robot,
        }
    }

    pub fn typed_request(&self) -> Result<Wave4ActionRequest, Wave4HostPortError> {
        parse_action_request(self.capability_id().as_ref(), self.input())
    }

    fn input(&self) -> &StrictJsonValue {
        match self {
            Self::ChannelReply { input }
            | Self::ChannelSend { input }
            | Self::CompanionSummon { input }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4ResourceReference {
    pub binding_id: ResourceBindingId,
    pub resource_kind: ResourceKind,
    pub resource_id: ResourceId,
    pub owner_id: String,
}

impl Wave4ResourceReference {
    fn from_binding(binding: &TypedResourceBinding) -> Self {
        Self {
            binding_id: binding.binding_id.clone(),
            resource_kind: binding.resource_kind.clone(),
            resource_id: binding.resource_id.clone(),
            owner_id: binding.owner_id.clone(),
        }
    }

    fn validate(&self) -> Result<(), Wave4HostPortError> {
        validate_bounded_identifier(
            "resource.binding_id",
            self.binding_id.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "resource.resource_kind",
            self.resource_kind.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "resource.resource_id",
            self.resource_id.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "resource.owner_id",
            &self.owner_id,
            MAX_REFERENCE_CHARS,
        )
    }
}

/// Stable replay identity owned by the Wave 4 domain repository.
///
/// Including the canonical request digest makes reuse of one idempotency key
/// with a different action payload a contract error instead of a second
/// effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4IdempotencyIdentity {
    pub principal_id: String,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_id: ResourceId,
    pub request_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wave4EffectRouteDescriptor {
    pub capability_id: &'static str,
    pub action_id: &'static str,
    pub owner_domain: Wave4OwnerDomain,
    pub resource_kind: &'static str,
    pub command_port_id: &'static str,
    pub outbox_port_id: &'static str,
}

pub fn effect_route_descriptor(
    capability_id: &str,
) -> Option<Wave4EffectRouteDescriptor> {
    let (
        canonical_capability_id,
        action_id,
        owner_domain,
        resource_kind,
        command_port_id,
        outbox_port_id,
    ) =
        match capability_id {
            CHANNEL_REPLY => (
                CHANNEL_REPLY,
                CHANNEL_REPLY_ACTION,
                Wave4OwnerDomain::Channel,
                CHANNEL_RESOURCE_KIND,
                CHANNEL_EFFECT_COMMAND_PORT_ID,
                CHANNEL_EFFECT_OUTBOX_PORT_ID,
            ),
            CHANNEL_SEND => (
                CHANNEL_SEND,
                CHANNEL_SEND_ACTION,
                Wave4OwnerDomain::Channel,
                CHANNEL_RESOURCE_KIND,
                CHANNEL_EFFECT_COMMAND_PORT_ID,
                CHANNEL_EFFECT_OUTBOX_PORT_ID,
            ),
            COMPANION_SUMMON => (
                COMPANION_SUMMON,
                COMPANION_SUMMON_ACTION,
                Wave4OwnerDomain::Companion,
                COMPANION_RESOURCE_KIND,
                COMPANION_EFFECT_COMMAND_PORT_ID,
                COMPANION_EFFECT_OUTBOX_PORT_ID,
            ),
            COMPANION_LEARN => (
                COMPANION_LEARN,
                COMPANION_LEARN_ACTION,
                Wave4OwnerDomain::Companion,
                COMPANION_MEMORY_RESOURCE_KIND,
                COMPANION_EFFECT_COMMAND_PORT_ID,
                COMPANION_EFFECT_OUTBOX_PORT_ID,
            ),
            COMPANION_EVOLVE => (
                COMPANION_EVOLVE,
                COMPANION_EVOLVE_ACTION,
                Wave4OwnerDomain::Companion,
                COMPANION_MEMORY_RESOURCE_KIND,
                COMPANION_EFFECT_COMMAND_PORT_ID,
                COMPANION_EFFECT_OUTBOX_PORT_ID,
            ),
            CUSTOMER_SERVICE_NOTES_READ => (
                CUSTOMER_SERVICE_NOTES_READ,
                CUSTOMER_SERVICE_NOTES_READ_ACTION,
                Wave4OwnerDomain::CustomerService,
                CUSTOMER_RESOURCE_KIND,
                CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID,
                CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID,
            ),
            CUSTOMER_SERVICE_NOTES_WRITE => (
                CUSTOMER_SERVICE_NOTES_WRITE,
                CUSTOMER_SERVICE_NOTES_WRITE_ACTION,
                Wave4OwnerDomain::CustomerService,
                CUSTOMER_RESOURCE_KIND,
                CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID,
                CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID,
            ),
            CUSTOMER_SERVICE_HANDOFF => (
                CUSTOMER_SERVICE_HANDOFF,
                CUSTOMER_SERVICE_HANDOFF_ACTION,
                Wave4OwnerDomain::CustomerService,
                CUSTOMER_RESOURCE_KIND,
                CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID,
                CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID,
            ),
            ROBOT_DISPLAY => (
                ROBOT_DISPLAY,
                ROBOT_DISPLAY_ACTION,
                Wave4OwnerDomain::Robot,
                ROBOT_RESOURCE_KIND,
                ROBOT_EFFECT_COMMAND_PORT_ID,
                ROBOT_EFFECT_OUTBOX_PORT_ID,
            ),
            ROBOT_MOTION => (
                ROBOT_MOTION,
                ROBOT_MOTION_ACTION,
                Wave4OwnerDomain::Robot,
                ROBOT_RESOURCE_KIND,
                ROBOT_EFFECT_COMMAND_PORT_ID,
                ROBOT_EFFECT_OUTBOX_PORT_ID,
            ),
            ROBOT_DEVICE_TOOLS => (
                ROBOT_DEVICE_TOOLS,
                ROBOT_DEVICE_TOOLS_ACTION,
                Wave4OwnerDomain::Robot,
                ROBOT_RESOURCE_KIND,
                ROBOT_EFFECT_COMMAND_PORT_ID,
                ROBOT_EFFECT_OUTBOX_PORT_ID,
            ),
            _ => return None,
        };
    Some(Wave4EffectRouteDescriptor {
        capability_id: canonical_capability_id,
        action_id,
        owner_domain,
        resource_kind,
        command_port_id,
        outbox_port_id,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4TypedCommandDescriptor {
    pub port_id: String,
    pub command_id: String,
    pub command_kind: String,
    pub contract_version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave4EffectCommand {
    pub context: Wave4HostContext,
    pub descriptor: Wave4TypedCommandDescriptor,
    pub identity: Wave4IdempotencyIdentity,
    pub resource: Wave4ResourceReference,
    pub request: Wave4ActionRequest,
}

impl Wave4EffectCommand {
    pub fn owner_domain(&self) -> Wave4OwnerDomain {
        self.request.owner_domain()
    }

    pub fn validate(&self) -> Result<(), Wave4HostPortError> {
        validate_host_context(&self.context)?;
        self.request.validate()?;
        let capability_id = self.request.capability_id();
        let action_id = self.request.action_id();
        if capability_id != self.context.capability_id
            || action_id != self.context.action_id
        {
            return Err(Wave4HostPortError::action_operation_mismatch(
                "typed effect command does not match its host context",
            ));
        }
        let route = effect_route_descriptor(capability_id.as_ref()).ok_or_else(|| {
            Wave4HostPortError::effect_contract_invalid(format!(
                "{} has no Wave 4 effect route",
                capability_id.as_ref()
            ))
        })?;
        if route.action_id != action_id.as_ref()
            || route.owner_domain != self.request.owner_domain()
            || route.resource_kind != self.resource.resource_kind.as_ref()
        {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "typed effect route, action, owner, or resource kind mismatch",
            ));
        }
        self.resource.validate()?;
        if self.resource.owner_id != self.context.principal.principal_id {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "typed effect resource owner does not match the principal",
            ));
        }
        let binding_matches = self.context.resource_bindings.iter().any(|binding| {
            binding.binding_id == self.resource.binding_id
                && binding.resource_kind == self.resource.resource_kind
                && binding.resource_id == self.resource.resource_id
                && binding.owner_id == self.resource.owner_id
        });
        if !binding_matches {
            return Err(Wave4HostPortError::resource_binding_invalid(
                "typed effect resource reference is not present in the host context",
            ));
        }

        let expected_identity = idempotency_identity(
            &self.context,
            &self.resource,
            request_input_for_digest(&self.request),
        )?;
        if self.identity != expected_identity {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "typed effect idempotency identity does not match the request",
            ));
        }
        validate_bounded_identifier(
            "identity.principal_id",
            &self.identity.principal_id,
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "identity.agent_session_id",
            self.identity.agent_session_id.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "identity.operation_id",
            self.identity.operation_id.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_bounded_identifier(
            "identity.idempotency_key",
            self.identity.idempotency_key.as_ref(),
            MAX_REFERENCE_CHARS,
        )?;
        validate_digest("identity.request_digest", &self.identity.request_digest)?;

        let expected_descriptor = Wave4TypedCommandDescriptor {
            port_id: route.command_port_id.to_owned(),
            command_id: self.context.operation_id.as_ref().to_owned(),
            command_kind: route.action_id.to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
        };
        if self.descriptor != expected_descriptor {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "typed command descriptor does not match the canonical action route",
            ));
        }
        validate_bounded_identifier(
            "command.command_id",
            &self.descriptor.command_id,
            MAX_REFERENCE_CHARS,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelReplyOutcome {
    pub delivery_ref: String,
    pub provider_message_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSendOutcome {
    pub delivery_ref: String,
    pub provider_message_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionSummonOutcome {
    pub summon_ref: String,
    pub context_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionLearnOutcome {
    pub memory_ref: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionEvolveOutcome {
    pub evolution_ref: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceNoteOutcome {
    pub note_ref: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceNotesReadOutcome {
    pub notes: Vec<CustomerServiceNoteOutcome>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceNotesWriteOutcome {
    pub note_ref: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerServiceHandoffOutcome {
    pub handoff_ref: String,
    pub destination_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotDisplayOutcome {
    pub effect_ref: String,
    pub frame_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotMotionOutcome {
    pub effect_ref: String,
    pub motion_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RobotDeviceToolsOutcome {
    pub effect_ref: String,
    pub result: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave4ActionOutcome {
    ChannelReply(ChannelReplyOutcome),
    ChannelSend(ChannelSendOutcome),
    CompanionSummon(CompanionSummonOutcome),
    CompanionLearn(CompanionLearnOutcome),
    CompanionEvolve(CompanionEvolveOutcome),
    CustomerServiceNotesRead(CustomerServiceNotesReadOutcome),
    CustomerServiceNotesWrite(CustomerServiceNotesWriteOutcome),
    CustomerServiceHandoff(CustomerServiceHandoffOutcome),
    RobotDisplay(RobotDisplayOutcome),
    RobotMotion(RobotMotionOutcome),
    RobotDeviceTools(RobotDeviceToolsOutcome),
}

impl Wave4ActionOutcome {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::ChannelReply(_) => CHANNEL_REPLY,
            Self::ChannelSend(_) => CHANNEL_SEND,
            Self::CompanionSummon(_) => COMPANION_SUMMON,
            Self::CompanionLearn(_) => COMPANION_LEARN,
            Self::CompanionEvolve(_) => COMPANION_EVOLVE,
            Self::CustomerServiceNotesRead(_) => CUSTOMER_SERVICE_NOTES_READ,
            Self::CustomerServiceNotesWrite(_) => CUSTOMER_SERVICE_NOTES_WRITE,
            Self::CustomerServiceHandoff(_) => CUSTOMER_SERVICE_HANDOFF,
            Self::RobotDisplay(_) => ROBOT_DISPLAY,
            Self::RobotMotion(_) => ROBOT_MOTION,
            Self::RobotDeviceTools(_) => ROBOT_DEVICE_TOOLS,
        })
    }

    fn validate(&self) -> Result<(), Wave4HostPortError> {
        match self {
            Self::ChannelReply(outcome) => {
                validate_reference("outcome.delivery_ref", &outcome.delivery_ref)?;
                validate_optional_reference(
                    "outcome.provider_message_ref",
                    outcome.provider_message_ref.as_deref(),
                )
            }
            Self::ChannelSend(outcome) => {
                validate_reference("outcome.delivery_ref", &outcome.delivery_ref)?;
                validate_optional_reference(
                    "outcome.provider_message_ref",
                    outcome.provider_message_ref.as_deref(),
                )
            }
            Self::CompanionSummon(outcome) => {
                validate_reference("outcome.summon_ref", &outcome.summon_ref)?;
                validate_digest("outcome.context_digest", &outcome.context_digest)
            }
            Self::CompanionLearn(outcome) => {
                validate_reference("outcome.memory_ref", &outcome.memory_ref)?;
                validate_positive_revision("outcome.revision", outcome.revision)
            }
            Self::CompanionEvolve(outcome) => {
                validate_reference("outcome.evolution_ref", &outcome.evolution_ref)?;
                validate_positive_revision("outcome.revision", outcome.revision)
            }
            Self::CustomerServiceNotesRead(outcome) => {
                if outcome.notes.len() > MAX_NOTE_RESULTS {
                    return Err(Wave4HostPortError::effect_contract_invalid(format!(
                        "outcome.notes exceeds {MAX_NOTE_RESULTS} entries"
                    )));
                }
                for note in &outcome.notes {
                    validate_reference("outcome.notes.note_ref", &note.note_ref)?;
                    validate_bounded_text(
                        "outcome.notes.content",
                        &note.content,
                        MAX_NOTE_CONTENT_CHARS,
                    )?;
                }
                validate_positive_revision("outcome.revision", outcome.revision)
            }
            Self::CustomerServiceNotesWrite(outcome) => {
                validate_reference("outcome.note_ref", &outcome.note_ref)?;
                validate_positive_revision("outcome.revision", outcome.revision)
            }
            Self::CustomerServiceHandoff(outcome) => {
                validate_reference("outcome.handoff_ref", &outcome.handoff_ref)?;
                validate_reference("outcome.destination_ref", &outcome.destination_ref)
            }
            Self::RobotDisplay(outcome) => {
                validate_reference("outcome.effect_ref", &outcome.effect_ref)?;
                if let Some(digest) = &outcome.frame_digest {
                    validate_digest("outcome.frame_digest", digest)?;
                }
                Ok(())
            }
            Self::RobotMotion(outcome) => {
                validate_reference("outcome.effect_ref", &outcome.effect_ref)?;
                validate_optional_reference(
                    "outcome.motion_ref",
                    outcome.motion_ref.as_deref(),
                )
            }
            Self::RobotDeviceTools(outcome) => {
                validate_reference("outcome.effect_ref", &outcome.effect_ref)?;
                if !outcome.result.0.is_object() {
                    return Err(Wave4HostPortError::effect_contract_invalid(
                        "robot.device_tools outcome result must be an object",
                    ));
                }
                if outcome.result.0.to_string().len() > MAX_DEVICE_RESULT_BYTES {
                    return Err(Wave4HostPortError::effect_contract_invalid(format!(
                        "robot.device_tools outcome exceeds {MAX_DEVICE_RESULT_BYTES} bytes"
                    )));
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave4EffectStatus {
    Succeeded,
    Failed,
    Uncertain,
    Reconciled,
}

impl Wave4EffectStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Reconciled => "reconciled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4EffectFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4EffectUncertainty {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave4ReconcileOutcome {
    ConfirmedSucceeded(Wave4ActionOutcome),
    ConfirmedFailed(Wave4EffectFailure),
    StillUncertain(Wave4EffectUncertainty),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave4ReconciledEffect {
    pub uncertain_receipt_id: String,
    pub outcome: Wave4ReconcileOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave4EffectDisposition {
    Succeeded(Wave4ActionOutcome),
    Failed(Wave4EffectFailure),
    Uncertain(Wave4EffectUncertainty),
    Reconciled(Wave4ReconciledEffect),
}

impl Wave4EffectDisposition {
    pub fn status(&self) -> Wave4EffectStatus {
        match self {
            Self::Succeeded(_) => Wave4EffectStatus::Succeeded,
            Self::Failed(_) => Wave4EffectStatus::Failed,
            Self::Uncertain(_) => Wave4EffectStatus::Uncertain,
            Self::Reconciled(_) => Wave4EffectStatus::Reconciled,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave4TypedOutboxDescriptor {
    pub port_id: String,
    pub event_id: String,
    pub cursor: String,
    pub event_kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave4EffectReceipt {
    pub receipt_id: String,
    pub identity: Wave4IdempotencyIdentity,
    pub resource: Wave4ResourceReference,
    pub command: Wave4TypedCommandDescriptor,
    pub outbox: Wave4TypedOutboxDescriptor,
    pub disposition: Wave4EffectDisposition,
}

impl Wave4EffectReceipt {
    pub fn status(&self) -> Wave4EffectStatus {
        self.disposition.status()
    }

    pub fn validate_for(
        &self,
        command: &Wave4EffectCommand,
    ) -> Result<(), Wave4HostPortError> {
        command.validate()?;
        validate_reference("receipt.receipt_id", &self.receipt_id)?;
        if self.identity != command.identity {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "effect receipt idempotency identity mismatch",
            ));
        }
        if self.resource != command.resource {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "effect receipt resource reference mismatch",
            ));
        }
        if self.command != command.descriptor {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "effect receipt command descriptor mismatch",
            ));
        }
        let route = effect_route_descriptor(command.request.capability_id().as_ref())
            .expect("validated Wave 4 command has a route");
        if self.outbox.port_id != route.outbox_port_id {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "effect receipt outbox port does not match the action route",
            ));
        }
        validate_reference("receipt.outbox.event_id", &self.outbox.event_id)?;
        validate_bounded_identifier(
            "receipt.outbox.cursor",
            &self.outbox.cursor,
            MAX_OUTBOX_CURSOR_CHARS,
        )?;
        let expected_event_kind = format!(
            "{}.effect.{}",
            command.request.capability_id().as_ref(),
            self.status().as_str()
        );
        if self.outbox.event_kind != expected_event_kind {
            return Err(Wave4HostPortError::effect_contract_invalid(
                "effect receipt outbox event kind is not canonical",
            ));
        }
        self.validate_disposition(command.request.capability_id().as_ref())?;
        if self.to_strict_json().0.to_string().len() > MAX_EFFECT_RECEIPT_BYTES {
            return Err(Wave4HostPortError::effect_contract_invalid(format!(
                "effect receipt exceeds {MAX_EFFECT_RECEIPT_BYTES} bytes"
            )));
        }
        Ok(())
    }

    pub fn validate_transition_from(
        &self,
        previous: Option<&Wave4EffectReceipt>,
        command: &Wave4EffectCommand,
    ) -> Result<(), Wave4HostPortError> {
        self.validate_for(command)?;
        match (previous, &self.disposition) {
            (None, Wave4EffectDisposition::Reconciled(_)) => {
                Err(Wave4HostPortError::effect_transition_invalid(
                    "a reconciled receipt requires a prior uncertain receipt",
                ))
            }
            (None, _) => Ok(()),
            (
                Some(previous),
                Wave4EffectDisposition::Reconciled(reconciled),
            ) => {
                previous.validate_for(command)?;
                if previous.status() != Wave4EffectStatus::Uncertain {
                    return Err(Wave4HostPortError::effect_transition_invalid(
                        "only an uncertain effect may transition to reconciled",
                    ));
                }
                if previous.identity != self.identity
                    || previous.resource != self.resource
                    || previous.command != self.command
                {
                    return Err(Wave4HostPortError::effect_transition_invalid(
                        "effect reconciliation must retain identity, resource, and command",
                    ));
                }
                if reconciled.uncertain_receipt_id != previous.receipt_id {
                    return Err(Wave4HostPortError::effect_transition_invalid(
                        "effect reconciliation must reference the prior uncertain receipt",
                    ));
                }
                Ok(())
            }
            (Some(_), _) => Err(Wave4HostPortError::effect_transition_invalid(
                "a terminal Wave 4 effect may only transition from uncertain to reconciled",
            )),
        }
    }

    fn validate_disposition(
        &self,
        capability_id: &str,
    ) -> Result<(), Wave4HostPortError> {
        match &self.disposition {
            Wave4EffectDisposition::Succeeded(outcome) => {
                validate_outcome_for(capability_id, outcome)
            }
            Wave4EffectDisposition::Failed(failure) => {
                validate_effect_problem("failure", &failure.code, &failure.message)
            }
            Wave4EffectDisposition::Uncertain(uncertainty) => validate_effect_problem(
                "uncertainty",
                &uncertainty.code,
                &uncertainty.message,
            ),
            Wave4EffectDisposition::Reconciled(reconciled) => {
                validate_reference(
                    "reconciliation.uncertain_receipt_id",
                    &reconciled.uncertain_receipt_id,
                )?;
                match &reconciled.outcome {
                    Wave4ReconcileOutcome::ConfirmedSucceeded(outcome) => {
                        validate_outcome_for(capability_id, outcome)
                    }
                    Wave4ReconcileOutcome::ConfirmedFailed(failure) => {
                        validate_effect_problem(
                            "reconciliation.failure",
                            &failure.code,
                            &failure.message,
                        )
                    }
                    Wave4ReconcileOutcome::StillUncertain(uncertainty) => {
                        validate_effect_problem(
                            "reconciliation.uncertainty",
                            &uncertainty.code,
                            &uncertainty.message,
                        )
                    }
                }
            }
        }
    }

    pub fn to_strict_json(&self) -> StrictJsonValue {
        effect_receipt_json(self)
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
        )?;
        self.operation.typed_request()?;
        Ok(())
    }

    pub fn effect_command(&self) -> Result<Wave4EffectCommand, Wave4HostPortError> {
        self.validate()?;
        let request = self.operation.typed_request()?;
        let route = effect_route_descriptor(request.capability_id().as_ref())
            .expect("validated Wave 4 action has an effect route");
        let binding = self
            .context
            .resource_bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == route.resource_kind)
            .ok_or_else(|| {
                Wave4HostPortError::resource_binding_invalid(format!(
                    "{} is missing canonical resource kind {}",
                    request.capability_id().as_ref(),
                    route.resource_kind
                ))
            })?;
        let resource = Wave4ResourceReference::from_binding(binding);
        let identity = idempotency_identity(
            &self.context,
            &resource,
            request_input_for_digest(&request),
        )?;
        let descriptor = Wave4TypedCommandDescriptor {
            port_id: route.command_port_id.to_owned(),
            command_id: self.context.operation_id.as_ref().to_owned(),
            command_kind: route.action_id.to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
        };
        let command = Wave4EffectCommand {
            context: self.context.clone(),
            descriptor,
            identity,
            resource,
            request,
        };
        command.validate()?;
        Ok(command)
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

    pub fn resource_owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE4_RESOURCE_OWNER_MISMATCH, message)
    }

    pub fn effect_contract_invalid(message: impl Into<String>) -> Self {
        Self::new(WAVE4_EFFECT_CONTRACT_INVALID, message)
    }

    pub fn effect_transition_invalid(message: impl Into<String>) -> Self {
        Self::new(WAVE4_EFFECT_TRANSITION_INVALID, message)
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

/// Typed production owner for one of the four Wave 4 domains.
///
/// The owner must persist its idempotency/effect fact and domain outbox before
/// returning a receipt. The adapter validates that receipt and never converts
/// an acknowledgement or echoed request into success.
pub trait Wave4EffectOwner: Send + Sync {
    fn execute<'a>(
        &'a self,
        command: Wave4EffectCommand,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Wave4EffectReceipt, Wave4HostPortError>>
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
    pub channel: Option<Arc<dyn Wave4EffectOwner>>,
    pub companion: Option<Arc<dyn Wave4EffectOwner>>,
    pub customer_service: Option<Arc<dyn Wave4EffectOwner>>,
    pub robot: Option<Arc<dyn Wave4EffectOwner>>,
}

impl Wave4OwnerBindings {
    pub fn with_channel(mut self, owner: Arc<dyn Wave4EffectOwner>) -> Self {
        self.channel = Some(owner);
        self
    }

    pub fn with_companion(mut self, owner: Arc<dyn Wave4EffectOwner>) -> Self {
        self.companion = Some(owner);
        self
    }

    pub fn with_customer_service(
        mut self,
        owner: Arc<dyn Wave4EffectOwner>,
    ) -> Self {
        self.customer_service = Some(owner);
        self
    }

    pub fn with_robot(mut self, owner: Arc<dyn Wave4EffectOwner>) -> Self {
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

        let command = match request.effect_command() {
            Ok(command) => command,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let owner = match command.owner_domain() {
            Wave4OwnerDomain::Channel => self.bindings.channel.clone(),
            Wave4OwnerDomain::Companion => self.bindings.companion.clone(),
            Wave4OwnerDomain::CustomerService => self.bindings.customer_service.clone(),
            Wave4OwnerDomain::Robot => self.bindings.robot.clone(),
        };
        let capability_id = command.context.capability_id.clone();
        Box::pin(async move {
            let Some(owner) = owner else {
                return Err(Wave4HostPortError::unavailable(format!(
                    "no production owner is bound for {}",
                    capability_id.as_ref()
                )));
            };
            let receipt = owner.execute(command.clone()).await?;
            receipt.validate_for(&command)?;
            Ok(receipt.to_strict_json())
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
        resource_kinds: &[],
        requirements: &[],
        effect_class: None,
    },
    CapabilitySpec {
        id: CHANNEL_GROUP_POLICY,
        kind: CapabilityKind::TurnMiddleware,
        display_name: "Channel group policy",
        description: "Apply the owning channel's group policy to a turn.",
        resource_kinds: &[],
        requirements: &[],
        effect_class: None,
    },
];

const COMPANION_CAPABILITIES: [CapabilitySpec; 5] = [
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
        id: COMPANION_SUMMON,
        kind: CapabilityKind::Tool,
        display_name: "Companion summon",
        description: "Select an owned Companion for the current AgentSession.",
        resource_kinds: COMPANION_RESOURCE,
        requirements: &[ResourceRequirement {
            resource_kind: COMPANION_RESOURCE_KIND,
            operation: "read",
        }],
        effect_class: Some(EffectClass::ReadSensitive),
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
    command_ports: &[
        "channel.agent-session-command",
        "channel.inbound-receipt",
        CHANNEL_EFFECT_COMMAND_PORT_ID,
    ],
    outbox_ports: &[CHANNEL_EFFECT_OUTBOX_PORT_ID],
};
const COMPANION_PORTS: PortSpec = PortSpec {
    command_ports: &[
        "companion.agent-session-command",
        COMPANION_EFFECT_COMMAND_PORT_ID,
    ],
    outbox_ports: &[COMPANION_EFFECT_OUTBOX_PORT_ID],
};
const CUSTOMER_SERVICE_PORTS: PortSpec = PortSpec {
    command_ports: &[
        "customer-service.dialogue-command",
        "customer-service.handoff-command",
        CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID,
    ],
    outbox_ports: &[CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID],
};
const ROBOT_PORTS: PortSpec = PortSpec {
    command_ports: &["robot.agent-session-command", ROBOT_EFFECT_COMMAND_PORT_ID],
    outbox_ports: &[ROBOT_EFFECT_OUTBOX_PORT_ID],
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
        resource_binding(
            "wave4-channel",
            CHANNEL_RESOURCE_KIND,
            "channel",
            &["manage", "receive", "reply", "send"],
            &owner_id,
        ),
        resource_binding(
            "wave4-companion",
            COMPANION_RESOURCE_KIND,
            "companion",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "wave4-companion-memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            "companion-memory",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "wave4-customer",
            CUSTOMER_RESOURCE_KIND,
            "customer",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "wave4-robot",
            ROBOT_RESOURCE_KIND,
            "robot",
            &["audio", "display", "link", "motion", "vision"],
            &owner_id,
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

/// Build multiple typed bindings from caller-owned slot metadata.
pub fn typed_resource_bindings_for<'a>(
    owner_id: &str,
    entries: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a [&'a str])>,
) -> TypedResourceBindings {
    entries
        .into_iter()
        .map(|(binding_id, resource_kind, resource_id, operations)| {
            typed_resource_binding(
                binding_id,
                resource_kind,
                resource_id,
                owner_id,
                operations.iter().copied(),
            )
        })
        .collect()
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

/// Construct all five bundled Wave 4 registrations.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Construct all five bundled Wave 4 registrations with the host-owned
/// action port.
pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave4HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    PACKAGE_SPECS
        .iter()
        .map(|spec| registration_for(spec, Arc::clone(&action_host_port)))
        .collect()
}

pub fn channel_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[0], unconfigured_host_port())
}

pub fn companion_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[1], unconfigured_host_port())
}

pub fn customer_service_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[2], unconfigured_host_port())
}

pub fn robot_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[3], unconfigured_host_port())
}

pub fn notification_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[4], unconfigured_host_port())
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
        },
        contributions: PackageContributions {
            capabilities,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
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
    let host_port_bindings = if has_action_handler {
        declared_host_ports.insert(action_host_port_ref.id.clone());
        vec![host_port_binding()?]
    } else {
        Vec::new()
    };
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
        let output_schema = action_output_schema(spec.id);
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
        CapabilityKind::ContextContributor | CapabilityKind::TurnMiddleware => {
            let schema = object_schema(true);
            contributions
                .context_schema_refs
                .push(schema_ref(spec.id, "context", &schema)?);
        }
        CapabilityKind::EventSource | CapabilityKind::EventConsumer => {
            let schema = object_schema(true);
            contributions
                .event_schema_refs
                .push(schema_ref(spec.id, "event", &schema)?);
        }
        _ => {}
    }

    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.id),
        version: VersionString::from(CONTRACT_VERSION),
        kind: spec.kind,
        package: package.clone(),
        display: localized(spec.display_name, spec.description),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: AGENT_SURFACES
            .iter()
            .map(|surface| (*surface).to_owned())
            .collect(),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: object_schema(false),
        contributions,
    })
}

fn action_id_for(capability_id: &str) -> ActionId {
    ActionId::from(format!("{capability_id}.invoke"))
}

fn resource_binding(
    binding_id: &str,
    resource_kind: &str,
    resource_id: &str,
    operations: &[&str],
    owner_id: &str,
) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from(binding_id),
        resource_kind: ResourceKind::from(resource_kind),
        resource_id: ResourceId::from(resource_id),
        owner_id: owner_id.to_owned(),
        operations: operations
            .iter()
            .map(|operation| (*operation).to_owned())
            .collect(),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
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

fn object_schema(additional_properties: bool) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty_object always returns a JSON object");
    object.insert("type".to_owned(), "object".to_owned().into());
    object.insert(
        "additionalProperties".to_owned(),
        additional_properties.into(),
    );
    StrictJsonValue(value.0)
}

fn typed_object_schema(
    properties: Vec<(&str, StrictJsonValue)>,
    required: &[&str],
) -> StrictJsonValue {
    let mut schema = object_schema(false);
    let mut property_map = empty_object();
    {
        let properties_object = property_map
            .0
            .as_object_mut()
            .expect("empty object is a JSON object");
        for (name, property_schema) in properties {
            properties_object.insert(name.to_owned(), property_schema.0);
        }
    }
    let object = schema
        .0
        .as_object_mut()
        .expect("object schema is a JSON object");
    object.insert("properties".to_owned(), property_map.0);
    if !required.is_empty() {
        object.insert(
            "required".to_owned(),
            required
                .iter()
                .map(|field| (*field).to_owned())
                .collect::<Vec<_>>()
                .into(),
        );
    }
    schema
}

fn string_schema(min_length: u64, max_length: u64) -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("type".to_owned(), "string".into());
    object.insert("minLength".to_owned(), min_length.into());
    object.insert("maxLength".to_owned(), max_length.into());
    schema
}

fn unsigned_integer_schema(minimum: u64, maximum: u64) -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("type".to_owned(), "integer".into());
    object.insert("minimum".to_owned(), minimum.into());
    object.insert("maximum".to_owned(), maximum.into());
    schema
}

fn string_enum_schema(values: &[&str]) -> StrictJsonValue {
    let mut schema = string_schema(1, MAX_REFERENCE_CHARS as u64);
    schema
        .0
        .as_object_mut()
        .expect("string schema is a JSON object")
        .insert(
            "enum".to_owned(),
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
                .into(),
        );
    schema
}

fn array_schema(items: StrictJsonValue, max_items: usize) -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("type".to_owned(), "array".into());
    object.insert("items".to_owned(), items.0);
    object.insert("maxItems".to_owned(), (max_items as u64).into());
    schema
}

fn action_input_schema(capability_id: &str) -> StrictJsonValue {
    let reference = || string_schema(1, MAX_REFERENCE_CHARS as u64);
    let short_text = || string_schema(1, MAX_SHORT_TEXT_CHARS as u64);
    let message = || string_schema(1, MAX_MESSAGE_CHARS as u64);
    let content = || string_schema(1, MAX_CONTENT_CHARS as u64);
    match capability_id {
        CHANNEL_REPLY => typed_object_schema(
            vec![
                ("message_ref", reference()),
                ("text", message()),
            ],
            &["message_ref", "text"],
        ),
        CHANNEL_SEND => typed_object_schema(
            vec![
                ("destination_ref", reference()),
                ("text", message()),
            ],
            &["destination_ref", "text"],
        ),
        COMPANION_SUMMON => typed_object_schema(
            vec![
                (
                    "memory_refs",
                    array_schema(reference(), MAX_MEMORY_REFS),
                ),
                ("purpose", short_text()),
            ],
            &[],
        ),
        COMPANION_LEARN => typed_object_schema(
            vec![("content", content()), ("source_ref", reference())],
            &["content"],
        ),
        COMPANION_EVOLVE => typed_object_schema(
            vec![
                ("reason", message()),
                (
                    "expected_revision",
                    unsigned_integer_schema(1, u64::MAX),
                ),
            ],
            &["reason"],
        ),
        CUSTOMER_SERVICE_NOTES_READ => {
            let mut schema = typed_object_schema(
                vec![
                    ("note_ref", reference()),
                    ("query", short_text()),
                    (
                        "limit",
                        unsigned_integer_schema(1, MAX_NOTE_RESULTS as u64),
                    ),
                ],
                &[],
            );
            let note_variant = required_fields_constraint(&["note_ref"]);
            let query_variant = required_fields_constraint(&["query"]);
            schema
                .0
                .as_object_mut()
                .expect("typed object schema")
                .insert(
                    "oneOf".to_owned(),
                    vec![note_variant.0, query_variant.0].into(),
                );
            schema
        }
        CUSTOMER_SERVICE_NOTES_WRITE => typed_object_schema(
            vec![
                ("note_ref", reference()),
                (
                    "expected_revision",
                    unsigned_integer_schema(1, u64::MAX),
                ),
                ("content", content()),
                ("kind", short_text()),
            ],
            &["content"],
        ),
        CUSTOMER_SERVICE_HANDOFF => typed_object_schema(
            vec![
                ("dialogue_ref", reference()),
                ("destination_ref", reference()),
                ("reason", message()),
            ],
            &["dialogue_ref", "destination_ref", "reason"],
        ),
        ROBOT_DISPLAY => typed_object_schema(
            vec![
                ("text", message()),
                (
                    "duration_ms",
                    unsigned_integer_schema(1, 600_000),
                ),
            ],
            &["text"],
        ),
        ROBOT_MOTION => typed_object_schema(
            vec![
                ("motion", short_text()),
                (
                    "duration_ms",
                    unsigned_integer_schema(1, 600_000),
                ),
                ("parameters", object_schema(true)),
            ],
            &["motion"],
        ),
        ROBOT_DEVICE_TOOLS => typed_object_schema(
            vec![
                ("tool_name", short_text()),
                ("arguments", object_schema(true)),
            ],
            &["tool_name", "arguments"],
        ),
        _ => object_schema(false),
    }
}

fn required_fields_constraint(required: &[&str]) -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("type".to_owned(), "object".into());
    object.insert(
        "required".to_owned(),
        required
            .iter()
            .map(|field| (*field).to_owned())
            .collect::<Vec<_>>()
            .into(),
    );
    schema
}

fn action_outcome_schema(capability_id: &str) -> StrictJsonValue {
    let reference = || string_schema(1, MAX_REFERENCE_CHARS as u64);
    let digest = || string_schema(64, 64);
    let revision = || unsigned_integer_schema(1, u64::MAX);
    match capability_id {
        CHANNEL_REPLY | CHANNEL_SEND => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("delivery_ref", reference()),
                ("provider_message_ref", reference()),
            ],
            &["capability_id", "delivery_ref"],
        ),
        COMPANION_SUMMON => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("summon_ref", reference()),
                ("context_digest", digest()),
            ],
            &["capability_id", "summon_ref", "context_digest"],
        ),
        COMPANION_LEARN => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("memory_ref", reference()),
                ("revision", revision()),
            ],
            &["capability_id", "memory_ref", "revision"],
        ),
        COMPANION_EVOLVE => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("evolution_ref", reference()),
                ("revision", revision()),
            ],
            &["capability_id", "evolution_ref", "revision"],
        ),
        CUSTOMER_SERVICE_NOTES_READ => {
            let note_schema = typed_object_schema(
                vec![
                    ("note_ref", reference()),
                    (
                        "content",
                        string_schema(1, MAX_NOTE_CONTENT_CHARS as u64),
                    ),
                ],
                &["note_ref", "content"],
            );
            typed_object_schema(
                vec![
                    ("capability_id", string_enum_schema(&[capability_id])),
                    ("notes", array_schema(note_schema, MAX_NOTE_RESULTS)),
                    ("revision", revision()),
                ],
                &["capability_id", "notes", "revision"],
            )
        }
        CUSTOMER_SERVICE_NOTES_WRITE => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("note_ref", reference()),
                ("revision", revision()),
            ],
            &["capability_id", "note_ref", "revision"],
        ),
        CUSTOMER_SERVICE_HANDOFF => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("handoff_ref", reference()),
                ("destination_ref", reference()),
            ],
            &["capability_id", "handoff_ref", "destination_ref"],
        ),
        ROBOT_DISPLAY => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("effect_ref", reference()),
                ("frame_digest", digest()),
            ],
            &["capability_id", "effect_ref"],
        ),
        ROBOT_MOTION => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("effect_ref", reference()),
                ("motion_ref", reference()),
            ],
            &["capability_id", "effect_ref"],
        ),
        ROBOT_DEVICE_TOOLS => typed_object_schema(
            vec![
                ("capability_id", string_enum_schema(&[capability_id])),
                ("effect_ref", reference()),
                ("result", object_schema(true)),
            ],
            &["capability_id", "effect_ref", "result"],
        ),
        _ => object_schema(false),
    }
}

fn effect_identity_schema() -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "principal_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "agent_session_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "operation_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "idempotency_key",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "capability_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "action_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "resource_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            ("request_digest", string_schema(64, 64)),
        ],
        &[
            "principal_id",
            "agent_session_id",
            "operation_id",
            "idempotency_key",
            "capability_id",
            "action_id",
            "resource_id",
            "request_digest",
        ],
    )
}

fn effect_resource_schema() -> StrictJsonValue {
    let reference = || string_schema(1, MAX_REFERENCE_CHARS as u64);
    typed_object_schema(
        vec![
            ("binding_id", reference()),
            ("resource_kind", reference()),
            ("resource_id", reference()),
            ("owner_id", reference()),
        ],
        &["binding_id", "resource_kind", "resource_id", "owner_id"],
    )
}

fn effect_command_descriptor_schema() -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "port_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "command_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "command_kind",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "contract_version",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
        ],
        &["port_id", "command_id", "command_kind", "contract_version"],
    )
}

fn effect_outbox_descriptor_schema() -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "port_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "event_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "cursor",
                string_schema(1, MAX_OUTBOX_CURSOR_CHARS as u64),
            ),
            (
                "event_kind",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
        ],
        &["port_id", "event_id", "cursor", "event_kind"],
    )
}

fn effect_problem_schema() -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "code",
                string_schema(1, MAX_EFFECT_ERROR_CODE_CHARS as u64),
            ),
            (
                "message",
                string_schema(1, MAX_EFFECT_ERROR_MESSAGE_CHARS as u64),
            ),
        ],
        &["code", "message"],
    )
}

fn reconciliation_schema(
    capability_id: Option<&str>,
) -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "uncertain_receipt_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "outcome_kind",
                string_enum_schema(&[
                    "confirmed_succeeded",
                    "confirmed_failed",
                    "still_uncertain",
                ]),
            ),
            (
                "outcome",
                capability_id
                    .map(action_outcome_schema)
                    .unwrap_or_else(|| object_schema(true)),
            ),
            ("failure", effect_problem_schema()),
            ("uncertainty", effect_problem_schema()),
        ],
        &["uncertain_receipt_id", "outcome_kind"],
    )
}

fn effect_receipt_envelope_schema(
    capability_id: Option<&str>,
) -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "receipt_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "status",
                string_enum_schema(&[
                    "succeeded",
                    "failed",
                    "uncertain",
                    "reconciled",
                ]),
            ),
            ("identity", effect_identity_schema()),
            ("resource", effect_resource_schema()),
            ("command", effect_command_descriptor_schema()),
            ("outbox", effect_outbox_descriptor_schema()),
            (
                "outcome",
                capability_id
                    .map(action_outcome_schema)
                    .unwrap_or_else(|| object_schema(true)),
            ),
            ("failure", effect_problem_schema()),
            ("uncertainty", effect_problem_schema()),
            ("reconciliation", reconciliation_schema(capability_id)),
        ],
        &[
            "receipt_id",
            "status",
            "identity",
            "resource",
            "command",
            "outbox",
        ],
    )
}

fn action_output_schema(capability_id: &str) -> StrictJsonValue {
    effect_receipt_envelope_schema(Some(capability_id))
}

fn effect_command_envelope_schema() -> StrictJsonValue {
    let request_schemas = [
        CHANNEL_REPLY,
        CHANNEL_SEND,
        COMPANION_SUMMON,
        COMPANION_LEARN,
        COMPANION_EVOLVE,
        CUSTOMER_SERVICE_NOTES_READ,
        CUSTOMER_SERVICE_NOTES_WRITE,
        CUSTOMER_SERVICE_HANDOFF,
        ROBOT_DISPLAY,
        ROBOT_MOTION,
        ROBOT_DEVICE_TOOLS,
    ]
    .into_iter()
    .map(|capability_id| action_input_schema(capability_id).0)
    .collect::<Vec<_>>();
    let mut request_schema = empty_object();
    request_schema
        .0
        .as_object_mut()
        .expect("empty object is a JSON object")
        .insert("oneOf".to_owned(), request_schemas.into());
    typed_object_schema(
        vec![
            ("descriptor", effect_command_descriptor_schema()),
            ("identity", effect_identity_schema()),
            ("resource", effect_resource_schema()),
            ("request", request_schema),
        ],
        &["descriptor", "identity", "resource", "request"],
    )
}

fn effect_outbox_event_schema() -> StrictJsonValue {
    typed_object_schema(
        vec![
            (
                "event_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            (
                "cursor",
                string_schema(1, MAX_OUTBOX_CURSOR_CHARS as u64),
            ),
            (
                "event_kind",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
            ("identity", effect_identity_schema()),
            ("resource", effect_resource_schema()),
            (
                "receipt_id",
                string_schema(1, MAX_REFERENCE_CHARS as u64),
            ),
        ],
        &[
            "event_id",
            "cursor",
            "event_kind",
            "identity",
            "resource",
            "receipt_id",
        ],
    )
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
    let request_schema = effect_command_envelope_schema();
    let response_schema = effect_receipt_envelope_schema(None);
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

fn command_port(id: &str) -> Result<TypedCommandPortDescriptor, String> {
    let is_effect_port = matches!(
        id,
        CHANNEL_EFFECT_COMMAND_PORT_ID
            | COMPANION_EFFECT_COMMAND_PORT_ID
            | CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID
            | ROBOT_EFFECT_COMMAND_PORT_ID
    );
    let command_schema = if is_effect_port {
        effect_command_envelope_schema()
    } else {
        object_schema(true)
    };
    let receipt_schema = if is_effect_port {
        effect_receipt_envelope_schema(None)
    } else {
        object_schema(true)
    };
    Ok(TypedCommandPortDescriptor {
        port: host_port(id),
        command_schema: schema_ref(id, "command", &command_schema)?,
        receipt_schema: schema_ref(id, "receipt", &receipt_schema)?,
    })
}

fn outbox_port(id: &str) -> Result<DomainOutboxPortDescriptor, String> {
    let is_effect_port = matches!(
        id,
        CHANNEL_EFFECT_OUTBOX_PORT_ID
            | COMPANION_EFFECT_OUTBOX_PORT_ID
            | CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID
            | ROBOT_EFFECT_OUTBOX_PORT_ID
    );
    let event_schema = if is_effect_port {
        effect_outbox_event_schema()
    } else {
        object_schema(true)
    };
    let cursor_schema = if is_effect_port {
        typed_object_schema(
            vec![(
                "cursor",
                string_schema(1, MAX_OUTBOX_CURSOR_CHARS as u64),
            )],
            &["cursor"],
        )
    } else {
        object_schema(true)
    };
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
            if !input.0.is_object() {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{} input must be a JSON object", self.capability_id.as_ref()),
                });
            }

            validate_resource_bindings(
                &self.capability_id,
                &context.principal.principal_id,
                self.requirements,
                &context.resource_bindings,
            )?;
            let operation = operation_from_input(&self.capability_id, input)?;
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
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?;

            self.host_port
                .invoke(request)
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })
        })
    }
}

/// Convert a canonical capability ID and its object payload into the only
/// typed operation variant accepted by the host port.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave4CapabilityOperation, KernelError> {
    parse_action_request(capability_id.as_ref(), &input).map_err(|error| {
        KernelError::CapabilityExecution {
            reason: error.to_string(),
        }
    })?;
    let operation = match capability_id.as_ref() {
        CHANNEL_REPLY => Wave4CapabilityOperation::ChannelReply { input },
        CHANNEL_SEND => Wave4CapabilityOperation::ChannelSend { input },
        COMPANION_SUMMON => Wave4CapabilityOperation::CompanionSummon { input },
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
    Ok(operation)
}

fn parse_action_request(
    capability_id: &str,
    input: &StrictJsonValue,
) -> Result<Wave4ActionRequest, Wave4HostPortError> {
    if !input.0.is_object() {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} input must be a JSON object"
        )));
    }

    let request = match capability_id {
        CHANNEL_REPLY => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["message_ref", "text"],
            )?;
            Wave4ActionRequest::ChannelReply(ChannelReplyRequest {
                message_ref: required_input_string(
                    capability_id,
                    input,
                    "message_ref",
                    MAX_REFERENCE_CHARS,
                )?,
                text: required_input_string(
                    capability_id,
                    input,
                    "text",
                    MAX_MESSAGE_CHARS,
                )?,
            })
        }
        CHANNEL_SEND => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["destination_ref", "text"],
            )?;
            Wave4ActionRequest::ChannelSend(ChannelSendRequest {
                destination_ref: required_input_string(
                    capability_id,
                    input,
                    "destination_ref",
                    MAX_REFERENCE_CHARS,
                )?,
                text: required_input_string(
                    capability_id,
                    input,
                    "text",
                    MAX_MESSAGE_CHARS,
                )?,
            })
        }
        COMPANION_SUMMON => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["memory_refs", "purpose"],
            )?;
            Wave4ActionRequest::CompanionSummon(CompanionSummonRequest {
                memory_refs: optional_input_string_array(
                    capability_id,
                    input,
                    "memory_refs",
                    MAX_MEMORY_REFS,
                    MAX_REFERENCE_CHARS,
                )?
                .unwrap_or_default(),
                purpose: optional_input_string(
                    capability_id,
                    input,
                    "purpose",
                    MAX_SHORT_TEXT_CHARS,
                )?,
            })
        }
        COMPANION_LEARN => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["content", "source_ref"],
            )?;
            Wave4ActionRequest::CompanionLearn(CompanionLearnRequest {
                content: required_input_string(
                    capability_id,
                    input,
                    "content",
                    MAX_CONTENT_CHARS,
                )?,
                source_ref: optional_input_string(
                    capability_id,
                    input,
                    "source_ref",
                    MAX_REFERENCE_CHARS,
                )?,
            })
        }
        COMPANION_EVOLVE => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["reason", "expected_revision"],
            )?;
            let expected_revision =
                optional_input_u64(capability_id, input, "expected_revision")?;
            if expected_revision == Some(0) {
                return Err(Wave4HostPortError::invalid_request(format!(
                    "{capability_id} field `expected_revision` must be positive"
                )));
            }
            Wave4ActionRequest::CompanionEvolve(CompanionEvolveRequest {
                reason: required_input_string(
                    capability_id,
                    input,
                    "reason",
                    MAX_MESSAGE_CHARS,
                )?,
                expected_revision,
            })
        }
        CUSTOMER_SERVICE_NOTES_READ => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["note_ref", "query", "limit"],
            )?;
            let note_ref = optional_input_string(
                capability_id,
                input,
                "note_ref",
                MAX_REFERENCE_CHARS,
            )?;
            let query = optional_input_string(
                capability_id,
                input,
                "query",
                MAX_SHORT_TEXT_CHARS,
            )?;
            if note_ref.is_some() == query.is_some() {
                return Err(Wave4HostPortError::invalid_request(format!(
                    "{capability_id} requires exactly one of `note_ref` or `query`"
                )));
            }
            let limit = optional_input_u64(capability_id, input, "limit")?
                .unwrap_or(10);
            if !(1..=MAX_NOTE_RESULTS as u64).contains(&limit) {
                return Err(Wave4HostPortError::invalid_request(format!(
                    "{capability_id} field `limit` must be between 1 and {MAX_NOTE_RESULTS}"
                )));
            }
            Wave4ActionRequest::CustomerServiceNotesRead(
                CustomerServiceNotesReadRequest {
                    note_ref,
                    query,
                    limit: limit as u16,
                },
            )
        }
        CUSTOMER_SERVICE_NOTES_WRITE => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["note_ref", "expected_revision", "content", "kind"],
            )?;
            let note_ref = optional_input_string(
                capability_id,
                input,
                "note_ref",
                MAX_REFERENCE_CHARS,
            )?;
            let expected_revision =
                optional_input_u64(capability_id, input, "expected_revision")?;
            let target = match (note_ref, expected_revision) {
                (None, None) => CustomerServiceNoteWriteTarget::Create,
                (Some(note_ref), Some(expected_revision))
                    if expected_revision > 0 =>
                {
                    CustomerServiceNoteWriteTarget::Update {
                        note_ref,
                        expected_revision,
                    }
                }
                _ => {
                    return Err(Wave4HostPortError::invalid_request(format!(
                        "{capability_id} update requires both `note_ref` and a positive `expected_revision`; create requires neither"
                    )));
                }
            };
            Wave4ActionRequest::CustomerServiceNotesWrite(
                CustomerServiceNotesWriteRequest {
                    target,
                    content: required_input_string(
                        capability_id,
                        input,
                        "content",
                        MAX_CONTENT_CHARS,
                    )?,
                    kind: optional_input_string(
                        capability_id,
                        input,
                        "kind",
                        MAX_SHORT_TEXT_CHARS,
                    )?,
                },
            )
        }
        CUSTOMER_SERVICE_HANDOFF => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["dialogue_ref", "destination_ref", "reason"],
            )?;
            Wave4ActionRequest::CustomerServiceHandoff(
                CustomerServiceHandoffRequest {
                    dialogue_ref: required_input_string(
                        capability_id,
                        input,
                        "dialogue_ref",
                        MAX_REFERENCE_CHARS,
                    )?,
                    destination_ref: required_input_string(
                        capability_id,
                        input,
                        "destination_ref",
                        MAX_REFERENCE_CHARS,
                    )?,
                    reason: required_input_string(
                        capability_id,
                        input,
                        "reason",
                        MAX_MESSAGE_CHARS,
                    )?,
                },
            )
        }
        ROBOT_DISPLAY => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["text", "duration_ms"],
            )?;
            let duration_ms =
                optional_input_u64(capability_id, input, "duration_ms")?;
            validate_optional_duration(capability_id, duration_ms)?;
            Wave4ActionRequest::RobotDisplay(RobotDisplayRequest {
                text: required_input_string(
                    capability_id,
                    input,
                    "text",
                    MAX_MESSAGE_CHARS,
                )?,
                duration_ms,
            })
        }
        ROBOT_MOTION => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["motion", "duration_ms", "parameters"],
            )?;
            let duration_ms =
                optional_input_u64(capability_id, input, "duration_ms")?;
            validate_optional_duration(capability_id, duration_ms)?;
            Wave4ActionRequest::RobotMotion(RobotMotionRequest {
                motion: required_input_string(
                    capability_id,
                    input,
                    "motion",
                    MAX_SHORT_TEXT_CHARS,
                )?,
                duration_ms,
                parameters: optional_input_object(
                    capability_id,
                    input,
                    "parameters",
                    MAX_DEVICE_RESULT_BYTES,
                )?,
            })
        }
        ROBOT_DEVICE_TOOLS => {
            validate_allowed_input_fields(
                capability_id,
                input,
                &["tool_name", "arguments"],
            )?;
            Wave4ActionRequest::RobotDeviceTools(RobotDeviceToolsRequest {
                tool_name: required_input_string(
                    capability_id,
                    input,
                    "tool_name",
                    MAX_SHORT_TEXT_CHARS,
                )?,
                arguments: required_input_object(
                    capability_id,
                    input,
                    "arguments",
                    MAX_DEVICE_RESULT_BYTES,
                )?,
            })
        }
        _ => {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "{capability_id} does not expose an action host operation"
            )));
        }
    };
    Ok(request)
}

fn validate_allowed_input_fields(
    capability_id: &str,
    input: &StrictJsonValue,
    allowed: &[&str],
) -> Result<(), Wave4HostPortError> {
    let object = input
        .0
        .as_object()
        .expect("caller checked the Wave 4 input object");
    if let Some(field) = object.keys().find(|field| {
        !allowed
            .iter()
            .any(|allowed_field| *allowed_field == field.as_str())
    }) {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} input contains unknown field `{field}`"
        )));
    }
    Ok(())
}

fn required_input_string(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
    max_chars: usize,
) -> Result<String, Wave4HostPortError> {
    optional_input_string(capability_id, input, field, max_chars)?.ok_or_else(
        || {
            Wave4HostPortError::invalid_request(format!(
                "{capability_id} requires non-empty `{field}`"
            ))
        },
    )
}

fn optional_input_string(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, Wave4HostPortError> {
    let Some(value) = input.0.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` must be a string"
        )));
    };
    validate_bounded_text(field, value, max_chars).map_err(|error| {
        Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` is invalid: {}",
            error.message
        ))
    })?;
    Ok(Some(value.to_owned()))
}

fn optional_input_u64(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
) -> Result<Option<u64>, Wave4HostPortError> {
    let Some(value) = input.0.get(field) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` must be an unsigned integer"
        ))
    })
}

fn optional_input_string_array(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Option<Vec<String>>, Wave4HostPortError> {
    let Some(value) = input.0.get(field) else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` must be an array"
        )));
    };
    if values.len() > max_items {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` exceeds {max_items} entries"
        )));
    }
    let mut output = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let Some(value) = value.as_str() else {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{capability_id} field `{field}` entries must be strings"
            )));
        };
        validate_bounded_text(field, value, max_chars).map_err(|error| {
            Wave4HostPortError::invalid_request(format!(
                "{capability_id} field `{field}` is invalid: {}",
                error.message
            ))
        })?;
        if !seen.insert(value.to_owned()) {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{capability_id} field `{field}` contains duplicate `{value}`"
            )));
        }
        output.push(value.to_owned());
    }
    Ok(Some(output))
}

fn required_input_object(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
    max_bytes: usize,
) -> Result<StrictJsonValue, Wave4HostPortError> {
    optional_input_object(capability_id, input, field, max_bytes)?.ok_or_else(
        || {
            Wave4HostPortError::invalid_request(format!(
                "{capability_id} requires object `{field}`"
            ))
        },
    )
}

fn optional_input_object(
    capability_id: &str,
    input: &StrictJsonValue,
    field: &str,
    max_bytes: usize,
) -> Result<Option<StrictJsonValue>, Wave4HostPortError> {
    let Some(value) = input.0.get(field) else {
        return Ok(None);
    };
    if !value.is_object() {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` must be an object"
        )));
    }
    if value.to_string().len() > max_bytes {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `{field}` exceeds {max_bytes} bytes"
        )));
    }
    Ok(Some(StrictJsonValue(value.clone())))
}

fn validate_optional_duration(
    capability_id: &str,
    duration_ms: Option<u64>,
) -> Result<(), Wave4HostPortError> {
    if duration_ms.is_some_and(|duration| !(1..=600_000).contains(&duration)) {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{capability_id} field `duration_ms` must be between 1 and 600000"
        )));
    }
    Ok(())
}

fn validate_bounded_identifier(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), Wave4HostPortError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} must be a canonical non-empty string"
        )));
    }
    if value.chars().count() > max_chars {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} exceeds {max_chars} characters"
        )));
    }
    Ok(())
}

fn validate_bounded_text(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), Wave4HostPortError> {
    if value.trim().is_empty() {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} must not be blank"
        )));
    }
    if value.chars().count() > max_chars {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} exceeds {max_chars} characters"
        )));
    }
    Ok(())
}

fn validate_reference(
    field: &str,
    value: &str,
) -> Result<(), Wave4HostPortError> {
    validate_bounded_identifier(field, value, MAX_REFERENCE_CHARS)
}

fn validate_optional_reference(
    field: &str,
    value: Option<&str>,
) -> Result<(), Wave4HostPortError> {
    value
        .map(|value| validate_reference(field, value))
        .transpose()
        .map(|_| ())
}

fn validate_digest(field: &str, value: &str) -> Result<(), Wave4HostPortError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} must be a canonical lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

fn validate_positive_revision(
    field: &str,
    revision: u64,
) -> Result<(), Wave4HostPortError> {
    if revision == 0 {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{field} must be positive"
        )));
    }
    Ok(())
}

fn validate_effect_problem(
    field: &str,
    code: &str,
    message: &str,
) -> Result<(), Wave4HostPortError> {
    validate_bounded_identifier(
        &format!("{field}.code"),
        code,
        MAX_EFFECT_ERROR_CODE_CHARS,
    )?;
    validate_bounded_text(
        &format!("{field}.message"),
        message,
        MAX_EFFECT_ERROR_MESSAGE_CHARS,
    )
}

fn validate_outcome_for(
    capability_id: &str,
    outcome: &Wave4ActionOutcome,
) -> Result<(), Wave4HostPortError> {
    if outcome.capability_id().as_ref() != capability_id {
        return Err(Wave4HostPortError::effect_contract_invalid(format!(
            "{capability_id} receipt contains outcome for {}",
            outcome.capability_id().as_ref()
        )));
    }
    outcome.validate()
}

fn idempotency_identity(
    context: &Wave4HostContext,
    resource: &Wave4ResourceReference,
    request: StrictJsonValue,
) -> Result<Wave4IdempotencyIdentity, Wave4HostPortError> {
    let request_digest = digest_payload(&request)
        .map_err(|error| {
            Wave4HostPortError::effect_contract_invalid(format!(
                "failed to digest typed Wave 4 request: {error}"
            ))
        })?
        .as_ref()
        .to_owned();
    Ok(Wave4IdempotencyIdentity {
        principal_id: context.principal.principal_id.clone(),
        agent_session_id: context.agent_session_id.clone(),
        operation_id: context.operation_id.clone(),
        idempotency_key: context.idempotency_key.clone(),
        capability_id: context.capability_id.clone(),
        action_id: context.action_id.clone(),
        resource_id: resource.resource_id.clone(),
        request_digest,
    })
}

fn request_input_for_digest(request: &Wave4ActionRequest) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    match request {
        Wave4ActionRequest::ChannelReply(request) => {
            object.insert(
                "message_ref".to_owned(),
                request.message_ref.clone().into(),
            );
            object.insert("text".to_owned(), request.text.clone().into());
        }
        Wave4ActionRequest::ChannelSend(request) => {
            object.insert(
                "destination_ref".to_owned(),
                request.destination_ref.clone().into(),
            );
            object.insert("text".to_owned(), request.text.clone().into());
        }
        Wave4ActionRequest::CompanionSummon(request) => {
            object.insert(
                "memory_refs".to_owned(),
                request.memory_refs.clone().into(),
            );
            if let Some(purpose) = &request.purpose {
                object.insert("purpose".to_owned(), purpose.clone().into());
            }
        }
        Wave4ActionRequest::CompanionLearn(request) => {
            object.insert("content".to_owned(), request.content.clone().into());
            if let Some(source_ref) = &request.source_ref {
                object.insert(
                    "source_ref".to_owned(),
                    source_ref.clone().into(),
                );
            }
        }
        Wave4ActionRequest::CompanionEvolve(request) => {
            object.insert("reason".to_owned(), request.reason.clone().into());
            if let Some(expected_revision) = request.expected_revision {
                object.insert(
                    "expected_revision".to_owned(),
                    expected_revision.into(),
                );
            }
        }
        Wave4ActionRequest::CustomerServiceNotesRead(request) => {
            if let Some(note_ref) = &request.note_ref {
                object.insert("note_ref".to_owned(), note_ref.clone().into());
            }
            if let Some(query) = &request.query {
                object.insert("query".to_owned(), query.clone().into());
            }
            object.insert("limit".to_owned(), u64::from(request.limit).into());
        }
        Wave4ActionRequest::CustomerServiceNotesWrite(request) => {
            match &request.target {
                CustomerServiceNoteWriteTarget::Create => {}
                CustomerServiceNoteWriteTarget::Update {
                    note_ref,
                    expected_revision,
                } => {
                    object.insert(
                        "note_ref".to_owned(),
                        note_ref.clone().into(),
                    );
                    object.insert(
                        "expected_revision".to_owned(),
                        (*expected_revision).into(),
                    );
                }
            }
            object.insert("content".to_owned(), request.content.clone().into());
            if let Some(kind) = &request.kind {
                object.insert("kind".to_owned(), kind.clone().into());
            }
        }
        Wave4ActionRequest::CustomerServiceHandoff(request) => {
            object.insert(
                "dialogue_ref".to_owned(),
                request.dialogue_ref.clone().into(),
            );
            object.insert(
                "destination_ref".to_owned(),
                request.destination_ref.clone().into(),
            );
            object.insert("reason".to_owned(), request.reason.clone().into());
        }
        Wave4ActionRequest::RobotDisplay(request) => {
            object.insert("text".to_owned(), request.text.clone().into());
            if let Some(duration_ms) = request.duration_ms {
                object.insert("duration_ms".to_owned(), duration_ms.into());
            }
        }
        Wave4ActionRequest::RobotMotion(request) => {
            object.insert("motion".to_owned(), request.motion.clone().into());
            if let Some(duration_ms) = request.duration_ms {
                object.insert("duration_ms".to_owned(), duration_ms.into());
            }
            if let Some(parameters) = &request.parameters {
                object.insert("parameters".to_owned(), parameters.0.clone());
            }
        }
        Wave4ActionRequest::RobotDeviceTools(request) => {
            object.insert(
                "tool_name".to_owned(),
                request.tool_name.clone().into(),
            );
            object.insert("arguments".to_owned(), request.arguments.0.clone());
        }
    }
    value
}

fn effect_receipt_json(receipt: &Wave4EffectReceipt) -> StrictJsonValue {
    let mut value = empty_object();
    {
        let object = value
            .0
            .as_object_mut()
            .expect("empty object is a JSON object");
        object.insert(
            "receipt_id".to_owned(),
            receipt.receipt_id.clone().into(),
        );
        object.insert(
            "status".to_owned(),
            receipt.status().as_str().to_owned().into(),
        );
        object.insert(
            "identity".to_owned(),
            idempotency_identity_json(&receipt.identity).0,
        );
        object.insert(
            "resource".to_owned(),
            resource_reference_json(&receipt.resource).0,
        );
        object.insert(
            "command".to_owned(),
            command_descriptor_json(&receipt.command).0,
        );
        object.insert(
            "outbox".to_owned(),
            outbox_descriptor_json(&receipt.outbox).0,
        );
        match &receipt.disposition {
            Wave4EffectDisposition::Succeeded(outcome) => {
                object.insert("outcome".to_owned(), action_outcome_json(outcome).0);
            }
            Wave4EffectDisposition::Failed(failure) => {
                object.insert("failure".to_owned(), effect_failure_json(failure).0);
            }
            Wave4EffectDisposition::Uncertain(uncertainty) => {
                object.insert(
                    "uncertainty".to_owned(),
                    effect_uncertainty_json(uncertainty).0,
                );
            }
            Wave4EffectDisposition::Reconciled(reconciled) => {
                object.insert(
                    "reconciliation".to_owned(),
                    reconciled_effect_json(reconciled).0,
                );
            }
        }
    }
    value
}

fn idempotency_identity_json(
    identity: &Wave4IdempotencyIdentity,
) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert(
        "principal_id".to_owned(),
        identity.principal_id.clone().into(),
    );
    object.insert(
        "agent_session_id".to_owned(),
        identity.agent_session_id.as_ref().to_owned().into(),
    );
    object.insert(
        "operation_id".to_owned(),
        identity.operation_id.as_ref().to_owned().into(),
    );
    object.insert(
        "idempotency_key".to_owned(),
        identity.idempotency_key.as_ref().to_owned().into(),
    );
    object.insert(
        "capability_id".to_owned(),
        identity.capability_id.as_ref().to_owned().into(),
    );
    object.insert(
        "action_id".to_owned(),
        identity.action_id.as_ref().to_owned().into(),
    );
    object.insert(
        "resource_id".to_owned(),
        identity.resource_id.as_ref().to_owned().into(),
    );
    object.insert(
        "request_digest".to_owned(),
        identity.request_digest.clone().into(),
    );
    value
}

fn resource_reference_json(
    resource: &Wave4ResourceReference,
) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert(
        "binding_id".to_owned(),
        resource.binding_id.as_ref().to_owned().into(),
    );
    object.insert(
        "resource_kind".to_owned(),
        resource.resource_kind.as_ref().to_owned().into(),
    );
    object.insert(
        "resource_id".to_owned(),
        resource.resource_id.as_ref().to_owned().into(),
    );
    object.insert("owner_id".to_owned(), resource.owner_id.clone().into());
    value
}

fn command_descriptor_json(
    descriptor: &Wave4TypedCommandDescriptor,
) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("port_id".to_owned(), descriptor.port_id.clone().into());
    object.insert(
        "command_id".to_owned(),
        descriptor.command_id.clone().into(),
    );
    object.insert(
        "command_kind".to_owned(),
        descriptor.command_kind.clone().into(),
    );
    object.insert(
        "contract_version".to_owned(),
        descriptor.contract_version.clone().into(),
    );
    value
}

fn outbox_descriptor_json(
    descriptor: &Wave4TypedOutboxDescriptor,
) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("port_id".to_owned(), descriptor.port_id.clone().into());
    object.insert("event_id".to_owned(), descriptor.event_id.clone().into());
    object.insert("cursor".to_owned(), descriptor.cursor.clone().into());
    object.insert(
        "event_kind".to_owned(),
        descriptor.event_kind.clone().into(),
    );
    value
}

fn action_outcome_json(outcome: &Wave4ActionOutcome) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert(
        "capability_id".to_owned(),
        outcome.capability_id().as_ref().to_owned().into(),
    );
    match outcome {
        Wave4ActionOutcome::ChannelReply(outcome) => {
            object.insert(
                "delivery_ref".to_owned(),
                outcome.delivery_ref.clone().into(),
            );
            if let Some(provider_message_ref) = &outcome.provider_message_ref {
                object.insert(
                    "provider_message_ref".to_owned(),
                    provider_message_ref.clone().into(),
                );
            }
        }
        Wave4ActionOutcome::ChannelSend(outcome) => {
            object.insert(
                "delivery_ref".to_owned(),
                outcome.delivery_ref.clone().into(),
            );
            if let Some(provider_message_ref) = &outcome.provider_message_ref {
                object.insert(
                    "provider_message_ref".to_owned(),
                    provider_message_ref.clone().into(),
                );
            }
        }
        Wave4ActionOutcome::CompanionSummon(outcome) => {
            object.insert(
                "summon_ref".to_owned(),
                outcome.summon_ref.clone().into(),
            );
            object.insert(
                "context_digest".to_owned(),
                outcome.context_digest.clone().into(),
            );
        }
        Wave4ActionOutcome::CompanionLearn(outcome) => {
            object.insert(
                "memory_ref".to_owned(),
                outcome.memory_ref.clone().into(),
            );
            object.insert("revision".to_owned(), outcome.revision.into());
        }
        Wave4ActionOutcome::CompanionEvolve(outcome) => {
            object.insert(
                "evolution_ref".to_owned(),
                outcome.evolution_ref.clone().into(),
            );
            object.insert("revision".to_owned(), outcome.revision.into());
        }
        Wave4ActionOutcome::CustomerServiceNotesRead(outcome) => {
            let notes = outcome
                .notes
                .iter()
                .map(|note| {
                    let mut note_value = empty_object();
                    let note_object = note_value
                        .0
                        .as_object_mut()
                        .expect("empty object is a JSON object");
                    note_object.insert(
                        "note_ref".to_owned(),
                        note.note_ref.clone().into(),
                    );
                    note_object.insert(
                        "content".to_owned(),
                        note.content.clone().into(),
                    );
                    note_value.0
                })
                .collect::<Vec<_>>();
            object.insert("notes".to_owned(), notes.into());
            object.insert("revision".to_owned(), outcome.revision.into());
        }
        Wave4ActionOutcome::CustomerServiceNotesWrite(outcome) => {
            object.insert(
                "note_ref".to_owned(),
                outcome.note_ref.clone().into(),
            );
            object.insert("revision".to_owned(), outcome.revision.into());
        }
        Wave4ActionOutcome::CustomerServiceHandoff(outcome) => {
            object.insert(
                "handoff_ref".to_owned(),
                outcome.handoff_ref.clone().into(),
            );
            object.insert(
                "destination_ref".to_owned(),
                outcome.destination_ref.clone().into(),
            );
        }
        Wave4ActionOutcome::RobotDisplay(outcome) => {
            object.insert(
                "effect_ref".to_owned(),
                outcome.effect_ref.clone().into(),
            );
            if let Some(frame_digest) = &outcome.frame_digest {
                object.insert(
                    "frame_digest".to_owned(),
                    frame_digest.clone().into(),
                );
            }
        }
        Wave4ActionOutcome::RobotMotion(outcome) => {
            object.insert(
                "effect_ref".to_owned(),
                outcome.effect_ref.clone().into(),
            );
            if let Some(motion_ref) = &outcome.motion_ref {
                object.insert(
                    "motion_ref".to_owned(),
                    motion_ref.clone().into(),
                );
            }
        }
        Wave4ActionOutcome::RobotDeviceTools(outcome) => {
            object.insert(
                "effect_ref".to_owned(),
                outcome.effect_ref.clone().into(),
            );
            object.insert("result".to_owned(), outcome.result.0.clone());
        }
    }
    value
}

fn effect_failure_json(failure: &Wave4EffectFailure) -> StrictJsonValue {
    effect_problem_json(&failure.code, &failure.message)
}

fn effect_uncertainty_json(
    uncertainty: &Wave4EffectUncertainty,
) -> StrictJsonValue {
    effect_problem_json(&uncertainty.code, &uncertainty.message)
}

fn effect_problem_json(code: &str, message: &str) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert("code".to_owned(), code.to_owned().into());
    object.insert("message".to_owned(), message.to_owned().into());
    value
}

fn reconciled_effect_json(
    reconciled: &Wave4ReconciledEffect,
) -> StrictJsonValue {
    let mut value = empty_object();
    let object = value
        .0
        .as_object_mut()
        .expect("empty object is a JSON object");
    object.insert(
        "uncertain_receipt_id".to_owned(),
        reconciled.uncertain_receipt_id.clone().into(),
    );
    match &reconciled.outcome {
        Wave4ReconcileOutcome::ConfirmedSucceeded(outcome) => {
            object.insert(
                "outcome_kind".to_owned(),
                "confirmed_succeeded".into(),
            );
            object.insert("outcome".to_owned(), action_outcome_json(outcome).0);
        }
        Wave4ReconcileOutcome::ConfirmedFailed(failure) => {
            object.insert(
                "outcome_kind".to_owned(),
                "confirmed_failed".into(),
            );
            object.insert("failure".to_owned(), effect_failure_json(failure).0);
        }
        Wave4ReconcileOutcome::StillUncertain(uncertainty) => {
            object.insert(
                "outcome_kind".to_owned(),
                "still_uncertain".into(),
            );
            object.insert(
                "uncertainty".to_owned(),
                effect_uncertainty_json(uncertainty).0,
            );
        }
    }
    value
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
            } else {
                KernelError::CapabilityExecution {
                    reason: error.to_string(),
                }
            }
        })
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
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
                "{} is missing resource kind {}",
                capability_id.as_ref(),
                requirement.resource_kind
            )));
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(Wave4HostPortError::resource_binding_invalid(format!(
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

    fn valid_action_input(capability_id: &str) -> StrictJsonValue {
        let mut input = empty_object();
        let object = input
            .0
            .as_object_mut()
            .expect("empty object");
        match capability_id {
            CHANNEL_REPLY => {
                object.insert("message_ref".to_owned(), "message-1".into());
                object.insert("text".to_owned(), "hello".into());
            }
            CHANNEL_SEND => {
                object.insert(
                    "destination_ref".to_owned(),
                    "destination-1".into(),
                );
                object.insert("text".to_owned(), "hello".into());
            }
            COMPANION_SUMMON => {}
            COMPANION_LEARN => {
                object.insert("content".to_owned(), "learn this".into());
            }
            COMPANION_EVOLVE => {
                object.insert("reason".to_owned(), "new evidence".into());
                object.insert("expected_revision".to_owned(), 1_u64.into());
            }
            CUSTOMER_SERVICE_NOTES_READ => {
                object.insert("query".to_owned(), "refund".into());
                object.insert("limit".to_owned(), 5_u64.into());
            }
            CUSTOMER_SERVICE_NOTES_WRITE => {
                object.insert("content".to_owned(), "refund policy".into());
            }
            CUSTOMER_SERVICE_HANDOFF => {
                object.insert("dialogue_ref".to_owned(), "dialogue-1".into());
                object.insert(
                    "destination_ref".to_owned(),
                    "queue-support".into(),
                );
                object.insert("reason".to_owned(), "human review".into());
            }
            ROBOT_DISPLAY => {
                object.insert("text".to_owned(), "hello".into());
                object.insert("duration_ms".to_owned(), 1_000_u64.into());
            }
            ROBOT_MOTION => {
                object.insert("motion".to_owned(), "wave".into());
                object.insert("parameters".to_owned(), empty_object().0);
            }
            ROBOT_DEVICE_TOOLS => {
                object.insert("tool_name".to_owned(), "read_sensor".into());
                object.insert("arguments".to_owned(), empty_object().0);
            }
            other => panic!("no action fixture for {other}"),
        }
        input
    }

    fn valid_operation(capability_id: &str) -> Wave4CapabilityOperation {
        operation_from_input(
            &CapabilityId::from(capability_id),
            valid_action_input(capability_id),
        )
        .expect("valid Wave 4 action fixture")
    }

    fn valid_outcome(capability_id: &str) -> Wave4ActionOutcome {
        match capability_id {
            CHANNEL_REPLY => Wave4ActionOutcome::ChannelReply(
                ChannelReplyOutcome {
                    delivery_ref: "delivery-1".to_owned(),
                    provider_message_ref: Some("provider-message-1".to_owned()),
                },
            ),
            CHANNEL_SEND => Wave4ActionOutcome::ChannelSend(
                ChannelSendOutcome {
                    delivery_ref: "delivery-1".to_owned(),
                    provider_message_ref: Some("provider-message-1".to_owned()),
                },
            ),
            COMPANION_SUMMON => Wave4ActionOutcome::CompanionSummon(
                CompanionSummonOutcome {
                    summon_ref: "summon-1".to_owned(),
                    context_digest: "a".repeat(64),
                },
            ),
            COMPANION_LEARN => Wave4ActionOutcome::CompanionLearn(
                CompanionLearnOutcome {
                    memory_ref: "memory-1".to_owned(),
                    revision: 1,
                },
            ),
            COMPANION_EVOLVE => Wave4ActionOutcome::CompanionEvolve(
                CompanionEvolveOutcome {
                    evolution_ref: "evolution-1".to_owned(),
                    revision: 1,
                },
            ),
            CUSTOMER_SERVICE_NOTES_READ => {
                Wave4ActionOutcome::CustomerServiceNotesRead(
                    CustomerServiceNotesReadOutcome {
                        notes: vec![CustomerServiceNoteOutcome {
                            note_ref: "note-1".to_owned(),
                            content: "refund policy".to_owned(),
                        }],
                        revision: 1,
                    },
                )
            }
            CUSTOMER_SERVICE_NOTES_WRITE => {
                Wave4ActionOutcome::CustomerServiceNotesWrite(
                    CustomerServiceNotesWriteOutcome {
                        note_ref: "note-1".to_owned(),
                        revision: 1,
                    },
                )
            }
            CUSTOMER_SERVICE_HANDOFF => {
                Wave4ActionOutcome::CustomerServiceHandoff(
                    CustomerServiceHandoffOutcome {
                        handoff_ref: "handoff-1".to_owned(),
                        destination_ref: "queue-support".to_owned(),
                    },
                )
            }
            ROBOT_DISPLAY => Wave4ActionOutcome::RobotDisplay(
                RobotDisplayOutcome {
                    effect_ref: "effect-1".to_owned(),
                    frame_digest: Some("b".repeat(64)),
                },
            ),
            ROBOT_MOTION => Wave4ActionOutcome::RobotMotion(
                RobotMotionOutcome {
                    effect_ref: "effect-1".to_owned(),
                    motion_ref: Some("motion-1".to_owned()),
                },
            ),
            ROBOT_DEVICE_TOOLS => {
                Wave4ActionOutcome::RobotDeviceTools(
                    RobotDeviceToolsOutcome {
                        effect_ref: "effect-1".to_owned(),
                        result: empty_object(),
                    },
                )
            }
            other => panic!("no outcome fixture for {other}"),
        }
    }

    fn valid_effect_command(capability_id: &str) -> Wave4EffectCommand {
        let route =
            effect_route_descriptor(capability_id).expect("effect route");
        valid_request(
            capability_id,
            route.action_id,
            route.resource_kind,
            valid_operation(capability_id),
        )
        .effect_command()
        .expect("valid effect command")
    }

    fn receipt_for(
        command: &Wave4EffectCommand,
        receipt_id: &str,
        disposition: Wave4EffectDisposition,
    ) -> Wave4EffectReceipt {
        let status = disposition.status();
        let route = effect_route_descriptor(
            command.request.capability_id().as_ref(),
        )
        .expect("effect route");
        Wave4EffectReceipt {
            receipt_id: receipt_id.to_owned(),
            identity: command.identity.clone(),
            resource: command.resource.clone(),
            command: command.descriptor.clone(),
            outbox: Wave4TypedOutboxDescriptor {
                port_id: route.outbox_port_id.to_owned(),
                event_id: format!("event-{receipt_id}"),
                cursor: format!("cursor-{receipt_id}"),
                event_kind: format!(
                    "{}.effect.{}",
                    command.request.capability_id().as_ref(),
                    status.as_str()
                ),
            },
            disposition,
        }
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
                    COMPANION_SUMMON.to_owned(),
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
            (COMPANION_SUMMON, CapabilityKind::Tool),
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
                manifest.entrypoint.entrypoint_profile,
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
                .any(|descriptor| descriptor.resource_kind == binding.resource_kind)
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
            let operation = valid_operation(capability.id);
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
                COMPANION_SUMMON,
                COMPANION_SUMMON_ACTION,
                Wave4OwnerDomain::Companion,
            ),
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
            let operation = valid_operation(capability_id);
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
                COMPANION_SUMMON => assert!(matches!(
                    operation,
                    Wave4CapabilityOperation::CompanionSummon { .. }
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
            let action_id = canonical_action_id(capability.id).expect("action identity");
            let operation = valid_operation(capability.id);
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
                input: valid_action_input(CHANNEL_REPLY),
            },
        );
        let error = mismatched.validate().expect_err("cross-capability operation must reject");
        assert_eq!(error.code, WAVE4_ACTION_OPERATION_MISMATCH);

        let pairing = Wave4HostRequest {
            context: valid_context(CHANNEL_PAIRING, "channel.pairing.invoke", CHANNEL_RESOURCE_KIND),
            operation: Wave4CapabilityOperation::ChannelReply {
                input: valid_action_input(CHANNEL_REPLY),
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
                input: valid_action_input(CHANNEL_REPLY),
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
    }

    #[test]
    fn composed_host_port_routes_only_to_injected_owner_and_keeps_missing_owner_unavailable() {
        fn poll_ready<F: Future>(future: F) -> F::Output {
            let waker = Waker::from(Arc::new(NoopWaker));
            let mut context = Context::from_waker(&waker);
            let mut future = Box::pin(future);
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("test host owner must settle immediately"),
            }
        }

        struct NoopWaker;

        impl Wake for NoopWaker {
            fn wake(self: Arc<Self>) {}
        }

        struct RejectingOwner {
            calls: Arc<Mutex<Vec<Wave4OwnerDomain>>>,
            domain: Wave4OwnerDomain,
        }

        impl Wave4EffectOwner for RejectingOwner {
            fn execute<'a>(
                &'a self,
                command: Wave4EffectCommand,
            ) -> Pin<
                Box<
                    dyn Future<
                            Output = Result<
                                Wave4EffectReceipt,
                                Wave4HostPortError,
                            >,
                        > + Send
                        + 'a,
                >,
            >
            {
                let calls = Arc::clone(&self.calls);
                let domain = self.domain;
                Box::pin(async move {
                    command.validate()?;
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
                input: valid_action_input(CHANNEL_REPLY),
            },
        );
        let error =
            poll_ready(host.invoke(request)).expect_err("boundary owner deliberately rejects");
        assert_eq!(error.code, "TEST_OWNER_REJECTED");
        assert_eq!(*calls.lock().unwrap(), vec![Wave4OwnerDomain::Channel]);

        let invalid = valid_request(
            CHANNEL_SEND,
            CHANNEL_SEND_ACTION,
            CHANNEL_RESOURCE_KIND,
            Wave4CapabilityOperation::ChannelReply {
                input: valid_action_input(CHANNEL_REPLY),
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
                input: valid_action_input(ROBOT_DISPLAY),
            },
        )))
        .expect_err("missing Robot owner must remain unavailable");
        assert_eq!(error.code, WAVE4_HOST_PORT_UNAVAILABLE);
    }

    #[test]
    fn channel_and_robot_availability_stays_on_host_execution_surfaces() {
        let expected_surfaces = AGENT_SURFACES
            .iter()
            .map(|surface| (*surface).to_owned())
            .collect::<BTreeSet<_>>();
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
            vec![
                "channel.agent-session-command",
                "channel.inbound-receipt",
                CHANNEL_EFFECT_COMMAND_PORT_ID,
            ]
        );
        assert_eq!(
            channel
                .metadata
                .context
                .domain_outbox_ports
                .iter()
                .map(|port| port.port.id.as_ref())
                .collect::<Vec<_>>(),
            vec![CHANNEL_EFFECT_OUTBOX_PORT_ID]
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
        assert!(pairing.contributions.host_ports.is_empty());
        assert!(!channel
            .handler_ids()
            .contains(&CapabilityId::from(CHANNEL_PAIRING)));
    }

    #[test]
    fn action_operation_projection_is_typed_and_has_no_pairing_branch() {
        assert!(matches!(
            operation_from_input(
                &CapabilityId::from(CHANNEL_REPLY),
                valid_action_input(CHANNEL_REPLY),
            ),
            Ok(Wave4CapabilityOperation::ChannelReply { .. })
        ));
        assert!(matches!(
            operation_from_input(
                &CapabilityId::from(ROBOT_DEVICE_TOOLS),
                valid_action_input(ROBOT_DEVICE_TOOLS),
            ),
            Ok(Wave4CapabilityOperation::RobotDeviceTools { .. })
        ));
        assert!(operation_from_input(
            &CapabilityId::from(CHANNEL_PAIRING),
            empty_object()
        )
        .is_err());
    }

    #[test]
    fn typed_effect_contract_covers_all_eleven_actions_and_routes() {
        let action_capabilities = all_capabilities()
            .filter(|capability| capability.effect_class.is_some())
            .collect::<Vec<_>>();
        assert_eq!(action_capabilities.len(), 11);

        let mut request_variants = BTreeSet::new();
        let mut command_ports = BTreeSet::new();
        let mut outbox_ports = BTreeSet::new();
        for capability in action_capabilities {
            let command = valid_effect_command(capability.id);
            command
                .validate()
                .unwrap_or_else(|error| panic!("{} command: {error}", capability.id));
            let route =
                effect_route_descriptor(capability.id).expect("effect route");
            assert_eq!(command.descriptor.port_id, route.command_port_id);
            assert_eq!(command.descriptor.command_kind, route.action_id);
            assert_eq!(
                command.resource.resource_kind.as_ref(),
                route.resource_kind
            );
            assert_eq!(command.owner_domain(), route.owner_domain);
            command_ports.insert(route.command_port_id);
            outbox_ports.insert(route.outbox_port_id);

            request_variants.insert(match &command.request {
                Wave4ActionRequest::ChannelReply(_) => "channel.reply",
                Wave4ActionRequest::ChannelSend(_) => "channel.send",
                Wave4ActionRequest::CompanionSummon(_) => "companion.summon",
                Wave4ActionRequest::CompanionLearn(_) => "companion.learn",
                Wave4ActionRequest::CompanionEvolve(_) => "companion.evolve",
                Wave4ActionRequest::CustomerServiceNotesRead(_) => {
                    "customer.notes.read"
                }
                Wave4ActionRequest::CustomerServiceNotesWrite(_) => {
                    "customer.notes.write"
                }
                Wave4ActionRequest::CustomerServiceHandoff(_) => {
                    "customer.handoff"
                }
                Wave4ActionRequest::RobotDisplay(_) => "robot.display",
                Wave4ActionRequest::RobotMotion(_) => "robot.motion",
                Wave4ActionRequest::RobotDeviceTools(_) => {
                    "robot.device_tools"
                }
            });

            let outcome = valid_outcome(capability.id);
            assert_eq!(outcome.capability_id().as_ref(), capability.id);
            let receipt = receipt_for(
                &command,
                &format!("receipt-{}", capability.id),
                Wave4EffectDisposition::Succeeded(outcome),
            );
            receipt
                .validate_transition_from(None, &command)
                .unwrap_or_else(|error| {
                    panic!("{} succeeded receipt: {error}", capability.id)
                });
            assert_eq!(
                receipt.to_strict_json().0["status"].as_str(),
                Some("succeeded")
            );
        }

        assert_eq!(request_variants.len(), 11);
        assert_eq!(
            command_ports,
            BTreeSet::from([
                CHANNEL_EFFECT_COMMAND_PORT_ID,
                COMPANION_EFFECT_COMMAND_PORT_ID,
                CUSTOMER_SERVICE_EFFECT_COMMAND_PORT_ID,
                ROBOT_EFFECT_COMMAND_PORT_ID,
            ])
        );
        assert_eq!(
            outbox_ports,
            BTreeSet::from([
                CHANNEL_EFFECT_OUTBOX_PORT_ID,
                COMPANION_EFFECT_OUTBOX_PORT_ID,
                CUSTOMER_SERVICE_EFFECT_OUTBOX_PORT_ID,
                ROBOT_EFFECT_OUTBOX_PORT_ID,
            ])
        );
    }

    #[test]
    fn effect_state_machine_rejects_illegal_transitions() {
        let command = valid_effect_command(CHANNEL_REPLY);
        let uncertainty = Wave4EffectUncertainty {
            code: "DELIVERY_UNKNOWN".to_owned(),
            message: "provider acknowledgement was not observed".to_owned(),
        };
        let uncertain = receipt_for(
            &command,
            "receipt-uncertain",
            Wave4EffectDisposition::Uncertain(uncertainty.clone()),
        );
        uncertain
            .validate_transition_from(None, &command)
            .expect("uncertain is a valid first terminal receipt");

        let reconciled = receipt_for(
            &command,
            "receipt-reconciled",
            Wave4EffectDisposition::Reconciled(Wave4ReconciledEffect {
                uncertain_receipt_id: uncertain.receipt_id.clone(),
                outcome: Wave4ReconcileOutcome::ConfirmedSucceeded(
                    valid_outcome(CHANNEL_REPLY),
                ),
            }),
        );
        reconciled
            .validate_transition_from(Some(&uncertain), &command)
            .expect("uncertain may reconcile with the same identity");

        let error = reconciled
            .validate_transition_from(None, &command)
            .expect_err("reconciled cannot be the first receipt");
        assert_eq!(error.code, WAVE4_EFFECT_TRANSITION_INVALID);

        let succeeded = receipt_for(
            &command,
            "receipt-succeeded",
            Wave4EffectDisposition::Succeeded(valid_outcome(CHANNEL_REPLY)),
        );
        let error = reconciled
            .validate_transition_from(Some(&succeeded), &command)
            .expect_err("succeeded cannot transition to reconciled");
        assert_eq!(error.code, WAVE4_EFFECT_TRANSITION_INVALID);

        let failed = receipt_for(
            &command,
            "receipt-failed",
            Wave4EffectDisposition::Failed(Wave4EffectFailure {
                code: "DELIVERY_REJECTED".to_owned(),
                message: "provider rejected the delivery".to_owned(),
            }),
        );
        let error = failed
            .validate_transition_from(Some(&uncertain), &command)
            .expect_err("uncertain cannot be overwritten by failed");
        assert_eq!(error.code, WAVE4_EFFECT_TRANSITION_INVALID);

        let mut wrong_reference = reconciled;
        let Wave4EffectDisposition::Reconciled(reconciliation) =
            &mut wrong_reference.disposition
        else {
            unreachable!("fixture is reconciled");
        };
        reconciliation.uncertain_receipt_id = "another-receipt".to_owned();
        let error = wrong_reference
            .validate_transition_from(Some(&uncertain), &command)
            .expect_err("reconciliation must name the prior uncertain receipt");
        assert_eq!(error.code, WAVE4_EFFECT_TRANSITION_INVALID);
    }

    #[test]
    fn effect_contract_rejects_owner_resource_and_descriptor_mismatch() {
        let command = valid_effect_command(CHANNEL_SEND);

        let mut wrong_identity = command.clone();
        wrong_identity.identity.principal_id = "foreign-owner".to_owned();
        let error = wrong_identity
            .validate()
            .expect_err("identity principal mismatch must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let mut wrong_owner = command.clone();
        wrong_owner.resource.owner_id = "foreign-owner".to_owned();
        let error = wrong_owner
            .validate()
            .expect_err("resource owner mismatch must reject");
        assert_eq!(error.code, WAVE4_RESOURCE_OWNER_MISMATCH);

        let mut wrong_resource = command.clone();
        wrong_resource.resource.resource_id =
            ResourceId::from("another-channel");
        let error = wrong_resource
            .validate()
            .expect_err("unbound resource reference must reject");
        assert_eq!(error.code, WAVE4_RESOURCE_BINDING_INVALID);

        let mut wrong_command_port = command.clone();
        wrong_command_port.descriptor.port_id =
            ROBOT_EFFECT_COMMAND_PORT_ID.to_owned();
        let error = wrong_command_port
            .validate()
            .expect_err("cross-domain command port must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let mut receipt = receipt_for(
            &command,
            "receipt-send",
            Wave4EffectDisposition::Succeeded(valid_outcome(CHANNEL_SEND)),
        );
        receipt.outbox.port_id = ROBOT_EFFECT_OUTBOX_PORT_ID.to_owned();
        let error = receipt
            .validate_for(&command)
            .expect_err("cross-domain outbox port must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let mut receipt = receipt_for(
            &command,
            "receipt-send",
            Wave4EffectDisposition::Succeeded(valid_outcome(CHANNEL_SEND)),
        );
        receipt.resource.resource_id = ResourceId::from("another-channel");
        let error = receipt
            .validate_for(&command)
            .expect_err("receipt resource mismatch must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);
    }

    #[test]
    fn effect_receipt_validation_is_bounded_and_action_specific() {
        let command = valid_effect_command(CHANNEL_REPLY);
        let mut receipt = receipt_for(
            &command,
            "receipt-reply",
            Wave4EffectDisposition::Succeeded(valid_outcome(CHANNEL_REPLY)),
        );
        receipt.receipt_id = "r".repeat(MAX_REFERENCE_CHARS + 1);
        let error = receipt
            .validate_for(&command)
            .expect_err("oversized receipt ID must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let mut receipt = receipt_for(
            &command,
            "receipt-reply",
            Wave4EffectDisposition::Succeeded(valid_outcome(CHANNEL_REPLY)),
        );
        receipt.outbox.cursor = "c".repeat(MAX_OUTBOX_CURSOR_CHARS + 1);
        let error = receipt
            .validate_for(&command)
            .expect_err("oversized outbox cursor must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let receipt = receipt_for(
            &command,
            "receipt-reply",
            Wave4EffectDisposition::Succeeded(valid_outcome(ROBOT_DISPLAY)),
        );
        let error = receipt
            .validate_for(&command)
            .expect_err("cross-action outcome must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let receipt = receipt_for(
            &command,
            "receipt-failed",
            Wave4EffectDisposition::Failed(Wave4EffectFailure {
                code: "DELIVERY_REJECTED".to_owned(),
                message: "x".repeat(MAX_EFFECT_ERROR_MESSAGE_CHARS + 1),
            }),
        );
        let error = receipt
            .validate_for(&command)
            .expect_err("oversized failure message must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let notes_command =
            valid_effect_command(CUSTOMER_SERVICE_NOTES_READ);
        let notes = (0..=MAX_NOTE_RESULTS)
            .map(|index| CustomerServiceNoteOutcome {
                note_ref: format!("note-{index}"),
                content: "bounded".to_owned(),
            })
            .collect();
        let receipt = receipt_for(
            &notes_command,
            "receipt-notes",
            Wave4EffectDisposition::Succeeded(
                Wave4ActionOutcome::CustomerServiceNotesRead(
                    CustomerServiceNotesReadOutcome { notes, revision: 1 },
                ),
            ),
        );
        let error = receipt
            .validate_for(&notes_command)
            .expect_err("oversized note result set must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);

        let device_command = valid_effect_command(ROBOT_DEVICE_TOOLS);
        let mut result = empty_object();
        result
            .0
            .as_object_mut()
            .expect("object")
            .insert(
                "payload".to_owned(),
                "x".repeat(MAX_DEVICE_RESULT_BYTES + 1).into(),
            );
        let receipt = receipt_for(
            &device_command,
            "receipt-device",
            Wave4EffectDisposition::Succeeded(
                Wave4ActionOutcome::RobotDeviceTools(
                    RobotDeviceToolsOutcome {
                        effect_ref: "effect-1".to_owned(),
                        result,
                    },
                ),
            ),
        );
        let error = receipt
            .validate_for(&device_command)
            .expect_err("oversized device result must reject");
        assert_eq!(error.code, WAVE4_EFFECT_CONTRACT_INVALID);
    }

    #[test]
    fn unconfigured_host_port_fails_closed_without_a_success_projection() {
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
                Poll::Pending => panic!("unconfigured host port must settle immediately"),
            }
        }

        let host_port = unconfigured_host_port();
        let result = poll_ready(host_port.invoke(Wave4HostRequest {
            context: Wave4HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "wave4-test-owner".to_owned(),
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
                capability_id: CapabilityId::from(CHANNEL_REPLY),
                action_id: ActionId::from(CHANNEL_REPLY_ACTION),
                state_scope_key: ScopeKey::from("session:wave4-test"),
                resource_bindings: canonical_resource_bindings("wave4-test-owner")
                    .into_iter()
                    .filter(|binding| binding.resource_kind.as_ref() == CHANNEL_RESOURCE_KIND)
                    .collect(),
            },
            operation: Wave4CapabilityOperation::ChannelReply {
                input: valid_action_input(CHANNEL_REPLY),
            },
        }));
        let error = result.expect_err("unconfigured host port must reject the action");
        assert_eq!(error.code, "WAVE4_HOST_PORT_UNAVAILABLE");
    }
}
