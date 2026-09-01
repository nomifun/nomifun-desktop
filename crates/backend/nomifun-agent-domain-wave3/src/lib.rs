//! Bundled creative and multimodal capability registrations for C7 Wave 3.
//!
//! The crate deliberately contains only contract metadata and typed host-backed
//! capability handlers.  Domain services are mounted by the shared
//! composition root; no application service bag or legacy route is required
//! to construct this inventory.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CancellationDescriptor, CanonicalSchemaRef,
    CapabilityActionDescriptor, CapabilityContributions, CapabilityId, CapabilityKind,
    CapabilityManifest, CorrelationId, EffectClass, HostPortBindingDescriptor, HostPortId,
    HostPortRef, IdempotencyKey, InProcessEntrypointMetadata, LocalizedMetadata,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor, PluginMountId,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    SkillId, StrictJsonValue, ToolPresentationKind, TypedResourceBinding, TypedResourceBindings,
    ValidatedPluginConfig, VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde_json::{Value, json};

pub const VERSION: &str = "1.0.0";
pub const CONTRACT_VERSION: &str = VERSION;
pub const PACKAGE_VERSION: &str = VERSION;

pub const CREATION_PACKAGE_ID: &str = "nomifun.creation";
pub const WORKSHOP_PACKAGE_ID: &str = "nomifun.workshop";
pub const OFFICE_PACKAGE_ID: &str = "nomifun.office";
pub const MINIAPP_PACKAGE_ID: &str = "nomifun.miniapp";

pub const CANVAS_RESOURCE_KIND: &str = "canvas";
pub const ASSET_LIBRARY_RESOURCE_KIND: &str = "asset_library";
pub const GENERATION_PROVIDER_RESOURCE_KIND: &str = "generation_provider";
pub const MINIAPP_RESOURCE_KIND: &str = "miniapp";

pub const TARGET_PACKAGE_IDS: [&str; 4] = [
    CREATION_PACKAGE_ID,
    WORKSHOP_PACKAGE_ID,
    OFFICE_PACKAGE_ID,
    MINIAPP_PACKAGE_ID,
];

pub const TARGET_CAPABILITY_IDS: [&str; 19] = [
    "creation.text",
    "creation.image",
    "creation.image_edit",
    "creation.video",
    "creation.audio",
    "workshop.canvas.read",
    "workshop.canvas.edit",
    "workshop.asset.read",
    "workshop.asset.write",
    "workshop.template.run",
    "workshop.director",
    "office.preview",
    "office.document.edit",
    "office.sheet.edit",
    "office.slides.edit",
    "miniapp.read",
    "miniapp.edit",
    "miniapp.publish",
    "miniapp.serve",
];

pub const PACKAGE_IDS: [&str; 4] = TARGET_PACKAGE_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 19] = TARGET_CAPABILITY_IDS;

/// The single host port for action-bearing Wave 3 capabilities.
///
/// The domain crate owns the capability vocabulary and resource requirements.
/// The application owns creation, Canvas, Office, and MiniApp facts and must
/// provide the adapter used by [`registrations_with_host_port`].
pub const WAVE3_CAPABILITY_HOST_PORT_ID: &str = "host.wave3.capability.invoke";

/// The resource slots frozen by the creative-studio official preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedResourceDescriptor {
    pub slot_key: &'static str,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: &'static str,
}

/// Invocation metadata projected from the Kernel into the Wave 3 host port.
///
/// No application service bag, Gateway state, legacy Conversation state, or
/// Kernel authority is exposed through this boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave3HostContext {
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

/// Typed domain-family operations accepted by the Wave 3 host.
///
/// Payload schemas remain owned by each capability.  The enum prevents the
/// registration crate from fabricating a result while allowing the owning
/// domain to validate and interpret its input.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave3CapabilityOperation {
    CreationText { input: StrictJsonValue },
    CreationImage { input: StrictJsonValue },
    CreationImageEdit { input: StrictJsonValue },
    CreationVideo { input: StrictJsonValue },
    CreationAudio { input: StrictJsonValue },
    WorkshopCanvasRead { input: StrictJsonValue },
    WorkshopCanvasEdit { input: StrictJsonValue },
    WorkshopAssetRead { input: StrictJsonValue },
    WorkshopAssetWrite { input: StrictJsonValue },
    WorkshopTemplateRun { input: StrictJsonValue },
    WorkshopDirector { input: StrictJsonValue },
    OfficePreview { input: StrictJsonValue },
    OfficeDocumentEdit { input: StrictJsonValue },
    OfficeSheetEdit { input: StrictJsonValue },
    OfficeSlidesEdit { input: StrictJsonValue },
    MiniAppRead { input: StrictJsonValue },
    MiniAppEdit { input: StrictJsonValue },
    MiniAppPublish { input: StrictJsonValue },
    MiniAppServe { input: StrictJsonValue },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave3HostRequest {
    pub context: Wave3HostContext,
    pub operation: Wave3CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave3HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave3HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new("WAVE3_HOST_PORT_UNAVAILABLE", message)
    }
}

impl fmt::Display for Wave3HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave3HostPortError {}

/// Production-owned implementation boundary for Wave 3 action execution.
///
/// Implementations must call the owning domain service and return its
/// canonical action result.  This trait deliberately has no successful
/// fallback implementation.
pub trait Wave3HostPort: Send + Sync {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>;
}

struct UnconfiguredWave3HostPort;

impl Wave3HostPort for UnconfiguredWave3HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            Err(Wave3HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

#[derive(Clone, Copy)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

#[derive(Clone, Copy)]
struct CapabilitySpec {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
    effect_class: EffectClass,
}

#[derive(Clone, Copy)]
struct PackageSpec {
    id: &'static str,
    mount_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    capabilities: &'static [CapabilitySpec],
}

const CREATION_TEXT_RESOURCES: &[&str] = &[GENERATION_PROVIDER_RESOURCE_KIND];
const CREATION_IMAGE_RESOURCES: &[&str] = &[GENERATION_PROVIDER_RESOURCE_KIND];
const CREATION_IMAGE_EDIT_RESOURCES: &[&str] = &[GENERATION_PROVIDER_RESOURCE_KIND];
const CREATION_VIDEO_RESOURCES: &[&str] = &[GENERATION_PROVIDER_RESOURCE_KIND];
const CREATION_AUDIO_RESOURCES: &[&str] = &[GENERATION_PROVIDER_RESOURCE_KIND];

const CREATION_TEXT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: GENERATION_PROVIDER_RESOURCE_KIND,
    operation: "text",
}];
const CREATION_IMAGE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: GENERATION_PROVIDER_RESOURCE_KIND,
    operation: "image",
}];
const CREATION_IMAGE_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: GENERATION_PROVIDER_RESOURCE_KIND,
    operation: "image",
}];
const CREATION_VIDEO_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: GENERATION_PROVIDER_RESOURCE_KIND,
    operation: "video",
}];
const CREATION_AUDIO_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: GENERATION_PROVIDER_RESOURCE_KIND,
    operation: "audio",
}];

const CANVAS_READ_RESOURCES: &[&str] = &[CANVAS_RESOURCE_KIND];
const CANVAS_EDIT_RESOURCES: &[&str] = &[CANVAS_RESOURCE_KIND];
const ASSET_READ_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];
const ASSET_WRITE_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];
const TEMPLATE_RUN_RESOURCES: &[&str] = &[CANVAS_RESOURCE_KIND];
const DIRECTOR_RESOURCES: &[&str] = &[CANVAS_RESOURCE_KIND];

const CANVAS_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CANVAS_RESOURCE_KIND,
    operation: "read",
}];
const CANVAS_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CANVAS_RESOURCE_KIND,
    operation: "write",
}];
const ASSET_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "read",
}];
const ASSET_WRITE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "write",
}];
const TEMPLATE_RUN_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CANVAS_RESOURCE_KIND,
    operation: "write",
}];
const DIRECTOR_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: CANVAS_RESOURCE_KIND,
    operation: "write",
}];

const OFFICE_PREVIEW_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];
const OFFICE_DOCUMENT_EDIT_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];
const OFFICE_SHEET_EDIT_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];
const OFFICE_SLIDES_EDIT_RESOURCES: &[&str] = &[ASSET_LIBRARY_RESOURCE_KIND];

const OFFICE_PREVIEW_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "read",
}];
const OFFICE_DOCUMENT_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "write",
}];
const OFFICE_SHEET_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "write",
}];
const OFFICE_SLIDES_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: ASSET_LIBRARY_RESOURCE_KIND,
    operation: "write",
}];

const MINIAPP_READ_RESOURCES: &[&str] = &[MINIAPP_RESOURCE_KIND];
const MINIAPP_EDIT_RESOURCES: &[&str] = &[MINIAPP_RESOURCE_KIND];
const MINIAPP_PUBLISH_RESOURCES: &[&str] = &[MINIAPP_RESOURCE_KIND];
const MINIAPP_SERVE_RESOURCES: &[&str] = &[MINIAPP_RESOURCE_KIND];

const MINIAPP_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: MINIAPP_RESOURCE_KIND,
    operation: "read",
}];
const MINIAPP_EDIT_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: MINIAPP_RESOURCE_KIND,
    operation: "edit",
}];
const MINIAPP_PUBLISH_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: MINIAPP_RESOURCE_KIND,
    operation: "publish",
}];
const MINIAPP_SERVE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: MINIAPP_RESOURCE_KIND,
    operation: "serve",
}];

const CREATION_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec {
        id: "creation.text",
        display_name: "Text creation",
        description: "Create bounded text output through the selected generation provider.",
        resource_kinds: CREATION_TEXT_RESOURCES,
        requirements: CREATION_TEXT_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
    CapabilitySpec {
        id: "creation.image",
        display_name: "Image creation",
        description: "Create an image artifact through the selected generation provider.",
        resource_kinds: CREATION_IMAGE_RESOURCES,
        requirements: CREATION_IMAGE_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
    CapabilitySpec {
        id: "creation.image_edit",
        display_name: "Image editing",
        description: "Create an edited image from an owned asset through the selected provider.",
        resource_kinds: CREATION_IMAGE_EDIT_RESOURCES,
        requirements: CREATION_IMAGE_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
    CapabilitySpec {
        id: "creation.video",
        display_name: "Video creation",
        description: "Create a video artifact through the selected generation provider.",
        resource_kinds: CREATION_VIDEO_RESOURCES,
        requirements: CREATION_VIDEO_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
    CapabilitySpec {
        id: "creation.audio",
        display_name: "Audio creation",
        description: "Create an audio artifact through the selected generation provider.",
        resource_kinds: CREATION_AUDIO_RESOURCES,
        requirements: CREATION_AUDIO_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
];

const WORKSHOP_CAPABILITIES: [CapabilitySpec; 6] = [
    CapabilitySpec {
        id: "workshop.canvas.read",
        display_name: "Read Canvas",
        description: "Read the selected Canvas revision and bounded graph.",
        resource_kinds: CANVAS_READ_RESOURCES,
        requirements: CANVAS_READ_REQUIREMENTS,
        effect_class: EffectClass::ReadSensitive,
    },
    CapabilitySpec {
        id: "workshop.canvas.edit",
        display_name: "Edit Canvas",
        description: "Apply a bounded edit to the selected Canvas revision.",
        resource_kinds: CANVAS_EDIT_RESOURCES,
        requirements: CANVAS_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteReversible,
    },
    CapabilitySpec {
        id: "workshop.asset.read",
        display_name: "Read asset",
        description: "Read metadata for an owned asset in the selected library.",
        resource_kinds: ASSET_READ_RESOURCES,
        requirements: ASSET_READ_REQUIREMENTS,
        effect_class: EffectClass::ReadSensitive,
    },
    CapabilitySpec {
        id: "workshop.asset.write",
        display_name: "Write asset",
        description: "Write an owned asset reference into the selected library.",
        resource_kinds: ASSET_WRITE_RESOURCES,
        requirements: ASSET_WRITE_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
    CapabilitySpec {
        id: "workshop.template.run",
        display_name: "Run template",
        description: "Run a selected Canvas template with owned assets and a provider.",
        resource_kinds: TEMPLATE_RUN_RESOURCES,
        requirements: TEMPLATE_RUN_REQUIREMENTS,
        effect_class: EffectClass::ExecuteLocal,
    },
    CapabilitySpec {
        id: "workshop.director",
        display_name: "Direct Canvas",
        description: "Apply a director operation to the selected Canvas and assets.",
        resource_kinds: DIRECTOR_RESOURCES,
        requirements: DIRECTOR_REQUIREMENTS,
        effect_class: EffectClass::WriteDurable,
    },
];

const OFFICE_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec {
        id: "office.preview",
        display_name: "Office preview",
        description: "Read a bounded office document preview from the selected workspace.",
        resource_kinds: OFFICE_PREVIEW_RESOURCES,
        requirements: OFFICE_PREVIEW_REQUIREMENTS,
        effect_class: EffectClass::ReadSensitive,
    },
    CapabilitySpec {
        id: "office.document.edit",
        display_name: "Edit document",
        description: "Apply a document edit in the selected workspace.",
        resource_kinds: OFFICE_DOCUMENT_EDIT_RESOURCES,
        requirements: OFFICE_DOCUMENT_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteReversible,
    },
    CapabilitySpec {
        id: "office.sheet.edit",
        display_name: "Edit sheet",
        description: "Apply a sheet edit in the selected workspace.",
        resource_kinds: OFFICE_SHEET_EDIT_RESOURCES,
        requirements: OFFICE_SHEET_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteReversible,
    },
    CapabilitySpec {
        id: "office.slides.edit",
        display_name: "Edit slides",
        description: "Apply a slides edit in the selected workspace.",
        resource_kinds: OFFICE_SLIDES_EDIT_RESOURCES,
        requirements: OFFICE_SLIDES_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteReversible,
    },
];

const MINIAPP_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec {
        id: "miniapp.read",
        display_name: "Read MiniApp",
        description: "Read the selected MiniApp source and published metadata.",
        resource_kinds: MINIAPP_READ_RESOURCES,
        requirements: MINIAPP_READ_REQUIREMENTS,
        effect_class: EffectClass::ReadSensitive,
    },
    CapabilitySpec {
        id: "miniapp.edit",
        display_name: "Edit MiniApp",
        description: "Apply an edit to the selected MiniApp working copy.",
        resource_kinds: MINIAPP_EDIT_RESOURCES,
        requirements: MINIAPP_EDIT_REQUIREMENTS,
        effect_class: EffectClass::WriteReversible,
    },
    CapabilitySpec {
        id: "miniapp.publish",
        display_name: "Publish MiniApp",
        description: "Publish the selected MiniApp snapshot.",
        resource_kinds: MINIAPP_PUBLISH_RESOURCES,
        requirements: MINIAPP_PUBLISH_REQUIREMENTS,
        effect_class: EffectClass::ExternalTransmit,
    },
    CapabilitySpec {
        id: "miniapp.serve",
        display_name: "Serve MiniApp",
        description: "Read the selected published MiniApp for serving.",
        resource_kinds: MINIAPP_SERVE_RESOURCES,
        requirements: MINIAPP_SERVE_REQUIREMENTS,
        effect_class: EffectClass::ExternalTransmit,
    },
];

const PACKAGE_SPECS: [PackageSpec; 4] = [
    PackageSpec {
        id: CREATION_PACKAGE_ID,
        mount_id: "domain-creation",
        display_name: "Creation",
        description: "Bundled multimodal creation capabilities.",
        capabilities: &CREATION_CAPABILITIES,
    },
    PackageSpec {
        id: WORKSHOP_PACKAGE_ID,
        mount_id: "domain-workshop",
        display_name: "Workshop",
        description: "Bundled Canvas, asset, template, and director capabilities.",
        capabilities: &WORKSHOP_CAPABILITIES,
    },
    PackageSpec {
        id: OFFICE_PACKAGE_ID,
        mount_id: "domain-office",
        display_name: "Office",
        description: "Bundled office preview and editing capabilities.",
        capabilities: &OFFICE_CAPABILITIES,
    },
    PackageSpec {
        id: MINIAPP_PACKAGE_ID,
        mount_id: "domain-miniapp",
        display_name: "MiniApp",
        description: "Bundled MiniApp read, edit, publish, and serve capabilities.",
        capabilities: &MINIAPP_CAPABILITIES,
    },
];

/// Return the four typed resource descriptors used by the creative preset.
pub fn typed_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    vec![
        descriptor(
            "canvas",
            CANVAS_RESOURCE_KIND,
            true,
            ["read", "write"],
            "require_explicit_selection",
        ),
        descriptor(
            "asset_library",
            ASSET_LIBRARY_RESOURCE_KIND,
            true,
            ["read", "write"],
            "select_only_owned_resource",
        ),
        descriptor(
            "generation_provider",
            GENERATION_PROVIDER_RESOURCE_KIND,
            false,
            ["audio", "image", "text", "video"],
            "select_only_owned_resource",
        ),
        descriptor(
            "miniapp",
            MINIAPP_RESOURCE_KIND,
            false,
            ["edit", "publish", "read", "serve"],
            "require_explicit_selection",
        ),
    ]
}

/// Return all resource descriptors owned by the creative slice.
pub fn all_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

/// Alias kept intentionally small for callers that use the generic term.
pub fn resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
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

/// Build the canonical creative resource bindings for one owner.
///
/// IDs are stable slot identities; callers provide the owner and may replace
/// the concrete resource IDs when constructing an AgentPreset revision.
pub fn canonical_resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    let owner_id = owner_id.into();
    vec![
        resource_binding(
            "creative-canvas",
            CANVAS_RESOURCE_KIND,
            "creative-canvas",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "creative-asset-library",
            ASSET_LIBRARY_RESOURCE_KIND,
            "creative-asset-library",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "creative-generation-provider",
            GENERATION_PROVIDER_RESOURCE_KIND,
            "creative-generation-provider",
            &["audio", "image", "text", "video"],
            &owner_id,
        ),
        resource_binding(
            "creative-miniapp",
            MINIAPP_RESOURCE_KIND,
            "creative-miniapp",
            &["edit", "publish", "read", "serve"],
            &owner_id,
        ),
    ]
}

/// Alias for callers that already use the contract's binding terminology.
pub fn resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    canonical_resource_bindings(owner_id)
}

/// Build the asset-library binding used by the Office capabilities.
pub fn office_asset_library_binding(owner_id: impl Into<String>) -> TypedResourceBinding {
    let owner_id = owner_id.into();
    resource_binding(
        "office-asset-library",
        ASSET_LIBRARY_RESOURCE_KIND,
        "office-asset-library",
        &["read", "write"],
        &owner_id,
    )
}

/// Return the resource kinds required by a capability in the canonical
/// inventory.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    find_capability(capability_id).map(|spec| {
        spec.resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
}

/// Construct the complete bundled Wave 3 registration inventory.
///
/// The default composition is metadata-only: action handlers are present so
/// the inventory can materialize, but invocation fails closed until the host
/// supplies a real domain adapter.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Return a host-port implementation that fails closed for unconfigured
/// metadata-only compositions and isolated contract tests.
pub fn unconfigured_host_port() -> Arc<dyn Wave3HostPort> {
    Arc::new(UnconfiguredWave3HostPort)
}

/// Construct the Wave 3 registration inventory with host-owned action
/// execution.
pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave3HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    PACKAGE_SPECS
        .iter()
        .map(|spec| registration_for(spec, Arc::clone(&action_host_port)))
        .collect()
}

pub fn creation_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[0], unconfigured_host_port())
}

pub fn workshop_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[1], unconfigured_host_port())
}

pub fn office_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[2], unconfigured_host_port())
}

pub fn miniapp_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGE_SPECS[3], unconfigured_host_port())
}

fn find_capability(capability_id: &str) -> Option<&'static CapabilitySpec> {
    PACKAGE_SPECS
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .find(|capability| capability.id == capability_id)
}

fn registration_for(
    spec: &PackageSpec,
    action_host_port: Arc<dyn Wave3HostPort>,
) -> Result<PluginRegistration, String> {
    let package = package_ref(spec.id);
    let config_schema = package_config_schema();
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| capability_manifest(&package, capability))
        .collect::<Result<Vec<_>, _>>()?;
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
    let action_host_port_ref = host_port(WAVE3_CAPABILITY_HOST_PORT_ID);
    let mut declared_host_ports =
        BTreeSet::from([cancellation_port.id.clone(), task_port.id.clone()]);
    declared_host_ports.insert(action_host_port_ref.id.clone());
    let context = PluginContextDescriptor {
        identity: identity.clone(),
        source: source.clone(),
        validated_config: ValidatedPluginConfig {
            schema_digest: digest_payload(&config_schema).map_err(|error| error.to_string())?,
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        state: PluginStateHandleDescriptor {
            package_id: PackageId::from(spec.id),
            mount_id: mount_id.clone(),
            methods: PluginStateMethod::REQUIRED.into_iter().collect(),
        },
        declared_services: Default::default(),
        host_ports: vec![host_port_binding()?],
        typed_command_ports: Vec::new(),
        domain_outbox_ports: Vec::new(),
        cancellation: CancellationDescriptor {
            cancellation_port: cancellation_port.clone(),
            scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
        },
        managed_task_registration: nomifun_agent_contracts::ManagedTaskRegistrationDescriptor {
            registrar_port: task_port.clone(),
            scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
        },
    };
    let manifest = PackageManifest {
        schema_version: VersionString::from(PACKAGE_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: PackageId::from(spec.id),
        package_version: VersionString::from(PACKAGE_VERSION),
        display: package_display(spec),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema,
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
            declared_skill_ids: BTreeSet::<SkillId>::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context,
    };
    let mut registration = PluginRegistration::new(metadata);
    for capability in spec.capabilities {
        registration
            .add_capability_handler(
                CapabilityId::from(capability.id),
                Arc::new(Wave3CapabilityHandler {
                    capability_id: CapabilityId::from(capability.id),
                    action_id: ActionId::from(capability.id),
                    requirements: capability.requirements,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(registration)
}

fn package_ref(package_id: &str) -> PackageRef {
    PackageRef {
        id: PackageId::from(package_id),
        version: VersionString::from(PACKAGE_VERSION),
    }
}

fn package_display(spec: &PackageSpec) -> LocalizedMetadata {
    LocalizedMetadata {
        name: spec.display_name.to_owned(),
        description: spec.description.to_owned(),
        localized_names: BTreeMap::from([("zh-CN".to_owned(), spec.display_name.to_owned())]),
        localized_descriptions: BTreeMap::from([("zh-CN".to_owned(), spec.description.to_owned())]),
    }
}

fn capability_display(spec: &CapabilitySpec) -> LocalizedMetadata {
    LocalizedMetadata {
        name: spec.display_name.to_owned(),
        description: spec.description.to_owned(),
        localized_names: BTreeMap::from([("zh-CN".to_owned(), spec.display_name.to_owned())]),
        localized_descriptions: BTreeMap::from([("zh-CN".to_owned(), spec.description.to_owned())]),
    }
}

fn capability_manifest(
    package: &PackageRef,
    spec: &CapabilitySpec,
) -> Result<CapabilityManifest, String> {
    let input_schema = action_input_schema();
    let output_schema = action_output_schema();
    let input_digest = digest_payload(&input_schema).map_err(|error| error.to_string())?;
    let output_digest = digest_payload(&output_schema).map_err(|error| error.to_string())?;
    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.id),
        version: VersionString::from(PACKAGE_VERSION),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: capability_display(spec),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: BTreeSet::new(),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: capability_config_schema(),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(spec.id),
                input_schema: CanonicalSchemaRef::from(format!(
                    "schema://{}/input@1#{}",
                    spec.id,
                    input_digest.as_ref()
                )),
                output_schema: CanonicalSchemaRef::from(format!(
                    "schema://{}/output@1#{}",
                    spec.id,
                    output_digest.as_ref()
                )),
                effect_class: spec.effect_class,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            context_schema_refs: Vec::new(),
            event_schema_refs: Vec::new(),
            resource_kinds: spec
                .resource_kinds
                .iter()
                .map(|kind| ResourceKind::from(*kind))
                .collect(),
            host_ports: vec![host_port(WAVE3_CAPABILITY_HOST_PORT_ID)],
        },
    })
}

fn package_config_schema() -> StrictJsonValue {
    StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false
    }))
}

fn capability_config_schema() -> StrictJsonValue {
    StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false
    }))
}

fn action_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

fn action_output_schema() -> Value {
    // The owning domain defines the operation result. The registration only
    // constrains the wire to a JSON object; the host owns the result shape.
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn schema_ref(
    subject: &str,
    role: &str,
    schema: &Value,
) -> Result<CanonicalSchemaRef, String> {
    let digest = digest_payload(schema).map_err(|error| error.to_string())?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{subject}/{role}@1#{}",
        digest.as_ref()
    )))
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

fn host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = action_input_schema();
    let response_schema = action_output_schema();
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE3_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(
            WAVE3_CAPABILITY_HOST_PORT_ID,
            "request",
            &request_schema,
        )?,
        response_schema: schema_ref(
            WAVE3_CAPABILITY_HOST_PORT_ID,
            "response",
            &response_schema,
        )?,
    })
}

struct Wave3CapabilityHandler {
    capability_id: CapabilityId,
    action_id: ActionId,
    requirements: &'static [ResourceRequirement],
    host_port: Arc<dyn Wave3HostPort>,
}

impl CapabilityHandler for Wave3CapabilityHandler {
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

            self.host_port
                .invoke(Wave3HostRequest {
                    context: Wave3HostContext {
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
                })
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })
        })
    }
}

fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave3CapabilityOperation, KernelError> {
    let operation = match capability_id.as_ref() {
        "creation.text" => Wave3CapabilityOperation::CreationText { input },
        "creation.image" => Wave3CapabilityOperation::CreationImage { input },
        "creation.image_edit" => Wave3CapabilityOperation::CreationImageEdit { input },
        "creation.video" => Wave3CapabilityOperation::CreationVideo { input },
        "creation.audio" => Wave3CapabilityOperation::CreationAudio { input },
        "workshop.canvas.read" => Wave3CapabilityOperation::WorkshopCanvasRead { input },
        "workshop.canvas.edit" => Wave3CapabilityOperation::WorkshopCanvasEdit { input },
        "workshop.asset.read" => Wave3CapabilityOperation::WorkshopAssetRead { input },
        "workshop.asset.write" => Wave3CapabilityOperation::WorkshopAssetWrite { input },
        "workshop.template.run" => Wave3CapabilityOperation::WorkshopTemplateRun { input },
        "workshop.director" => Wave3CapabilityOperation::WorkshopDirector { input },
        "office.preview" => Wave3CapabilityOperation::OfficePreview { input },
        "office.document.edit" => Wave3CapabilityOperation::OfficeDocumentEdit { input },
        "office.sheet.edit" => Wave3CapabilityOperation::OfficeSheetEdit { input },
        "office.slides.edit" => Wave3CapabilityOperation::OfficeSlidesEdit { input },
        "miniapp.read" => Wave3CapabilityOperation::MiniAppRead { input },
        "miniapp.edit" => Wave3CapabilityOperation::MiniAppEdit { input },
        "miniapp.publish" => Wave3CapabilityOperation::MiniAppPublish { input },
        "miniapp.serve" => Wave3CapabilityOperation::MiniAppServe { input },
        other => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{other} does not expose an action host operation"),
            });
        }
    };
    Ok(operation)
}

fn validate_resource_bindings(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &[TypedResourceBinding],
) -> Result<(), KernelError> {
    let mut seen_binding_ids = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().is_empty() || binding.resource_id.as_ref().is_empty() {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires non-empty binding and resource IDs",
                    capability_id.as_ref()
                ),
            });
        }
        if !seen_binding_ids.insert(binding.binding_id.clone()) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} received duplicate resource binding {}",
                    capability_id.as_ref(),
                    binding.binding_id.as_ref()
                ),
            });
        }
        if binding.owner_id != principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
    }

    for requirement in requirements {
        let Some(binding) = bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == requirement.resource_kind)
        else {
            return Err(KernelError::CapabilityResourceNotBound {
                capability_id: capability_id.clone(),
                resource_kind: requirement.resource_kind.to_owned(),
            });
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires operation {} on {}",
                    capability_id.as_ref(),
                    requirement.operation,
                    requirement.resource_kind
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll, Waker};

    use super::*;
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
    };

    #[test]
    fn registrations_cover_the_four_wave3_packages_and_all_target_capabilities() {
        let registrations = registrations().expect("Wave 3 registrations are canonical");
        assert_eq!(registrations.len(), PACKAGE_IDS.len());

        let expected = BTreeMap::from([
            (
                CREATION_PACKAGE_ID,
                BTreeSet::from([
                    "creation.audio".to_owned(),
                    "creation.image".to_owned(),
                    "creation.image_edit".to_owned(),
                    "creation.text".to_owned(),
                    "creation.video".to_owned(),
                ]),
            ),
            (
                WORKSHOP_PACKAGE_ID,
                BTreeSet::from([
                    "workshop.asset.read".to_owned(),
                    "workshop.asset.write".to_owned(),
                    "workshop.canvas.edit".to_owned(),
                    "workshop.canvas.read".to_owned(),
                    "workshop.director".to_owned(),
                    "workshop.template.run".to_owned(),
                ]),
            ),
            (
                OFFICE_PACKAGE_ID,
                BTreeSet::from([
                    "office.document.edit".to_owned(),
                    "office.preview".to_owned(),
                    "office.sheet.edit".to_owned(),
                    "office.slides.edit".to_owned(),
                ]),
            ),
            (
                MINIAPP_PACKAGE_ID,
                BTreeSet::from([
                    "miniapp.edit".to_owned(),
                    "miniapp.publish".to_owned(),
                    "miniapp.read".to_owned(),
                    "miniapp.serve".to_owned(),
                ]),
            ),
        ]);

        let mut observed = BTreeMap::new();
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
            assert!(manifest.entrypoint.entrypoint_profile == "trusted-in-process");
            assert_eq!(
                registration.handler_ids().len(),
                manifest.contributions.capabilities.len()
            );
            for capability in &manifest.contributions.capabilities {
                assert_eq!(capability.kind, CapabilityKind::Tool);
                assert_eq!(capability.contributions.actions.len(), 1);
                assert_eq!(
                    capability.contributions.actions[0].action_id.as_ref(),
                    capability.id.as_ref()
                );
                assert_eq!(
                    capability.contributions.host_ports,
                    vec![host_port(WAVE3_CAPABILITY_HOST_PORT_ID)]
                );
            }
            assert!(registration
                .metadata
                .context
                .host_ports
                .iter()
                .any(|binding| {
                    binding.port.id == HostPortId::from(WAVE3_CAPABILITY_HOST_PORT_ID)
                }));
            assert!(registration
                .metadata
                .registrar
                .declared_host_ports
                .contains(&HostPortId::from(WAVE3_CAPABILITY_HOST_PORT_ID)));
            observed.insert(
                manifest.package_id.as_ref().to_owned(),
                manifest
                    .contributions
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.as_ref().to_owned())
                    .collect::<BTreeSet<_>>(),
            );
        }
        let expected = expected
            .into_iter()
            .map(|(package_id, capabilities)| (package_id.to_owned(), capabilities))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(observed, expected);
        assert_eq!(
            observed
                .values()
                .flat_map(|capabilities| capabilities.iter())
                .count(),
            ALL_CAPABILITY_IDS.len()
        );
    }

    #[test]
    fn registrations_pass_kernel_materialization_without_partial_publication() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("state persistence");
        let materialized = registry
            .replace_all(registrations().expect("registrations"))
            .expect("Wave 3 metadata must materialize");
        assert_eq!(materialized.packages.len(), 4);
        assert_eq!(materialized.capabilities.len(), ALL_CAPABILITY_IDS.len());
        assert_eq!(materialized.generation, 1);
    }

    #[test]
    fn creative_resource_descriptors_match_the_frozen_typed_slots() {
        let descriptors = typed_resource_descriptors();
        assert_eq!(descriptors.len(), 4);
        assert_eq!(
            descriptors
                .iter()
                .map(|descriptor| descriptor.resource_kind.as_ref())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                CANVAS_RESOURCE_KIND,
                ASSET_LIBRARY_RESOURCE_KIND,
                GENERATION_PROVIDER_RESOURCE_KIND,
                MINIAPP_RESOURCE_KIND,
            ])
        );
        let provider = descriptors
            .iter()
            .find(|descriptor| {
                descriptor.resource_kind.as_ref() == GENERATION_PROVIDER_RESOURCE_KIND
            })
            .expect("provider descriptor");
        assert_eq!(
            provider.operations,
            BTreeSet::from([
                "audio".to_owned(),
                "image".to_owned(),
                "text".to_owned(),
                "video".to_owned(),
            ])
        );
        let bindings = canonical_resource_bindings("owner-1");
        assert_eq!(bindings.len(), 4);
        assert!(bindings.iter().all(|binding| binding.owner_id == "owner-1"));
        assert!(bindings.iter().all(|binding| {
            descriptors
                .iter()
                .any(|descriptor| descriptor.resource_kind == binding.resource_kind)
        }));
        assert_eq!(
            office_asset_library_binding("owner-1")
                .resource_kind
                .as_ref(),
            ASSET_LIBRARY_RESOURCE_KIND
        );
    }

    #[test]
    fn every_action_preserves_its_effect_class_in_metadata() {
        let registrations = registrations().expect("registrations");
        let actions = registrations
            .iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
                    .flat_map(|capability| capability.contributions.actions.iter())
            })
            .collect::<Vec<_>>();
        assert_eq!(actions.len(), ALL_CAPABILITY_IDS.len());
        assert!(actions.iter().all(|action| {
            !matches!(action.effect_class, EffectClass::Pure)
                || action.presentation == ToolPresentationKind::FunctionTool
        }));
    }

    #[test]
    fn every_action_capability_maps_to_a_typed_host_operation() {
        for capability in PACKAGE_SPECS
            .iter()
            .flat_map(|package| package.capabilities.iter())
        {
            let capability_id = CapabilityId::from(capability.id);
            assert!(
                operation_from_input(&capability_id, StrictJsonValue(serde_json::json!({}))).is_ok(),
                "{} must have a host operation",
                capability.id
            );
        }
    }

    #[test]
    fn operation_mapping_and_resource_requirements_match_the_frozen_inventory() {
        let bindings = canonical_resource_bindings("wave3-test-owner");

        for capability in PACKAGE_SPECS
            .iter()
            .flat_map(|package| package.capabilities.iter())
        {
            let capability_id = CapabilityId::from(capability.id);
            let operation = operation_from_input(&capability_id, StrictJsonValue(json!({})))
                .expect("every Wave 3 capability has a typed operation");

            match capability.id {
                "creation.text" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationText { .. }
                    ));
                }
                "creation.image" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationImage { .. }
                    ));
                }
                "creation.image_edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationImageEdit { .. }
                    ));
                }
                "creation.video" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationVideo { .. }
                    ));
                }
                "creation.audio" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationAudio { .. }
                    ));
                }
                "workshop.canvas.read" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopCanvasRead { .. }
                    ));
                }
                "workshop.canvas.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopCanvasEdit { .. }
                    ));
                }
                "workshop.asset.read" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopAssetRead { .. }
                    ));
                }
                "workshop.asset.write" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopAssetWrite { .. }
                    ));
                }
                "workshop.template.run" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopTemplateRun { .. }
                    ));
                }
                "workshop.director" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopDirector { .. }
                    ));
                }
                "office.preview" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::OfficePreview { .. }
                    ));
                }
                "office.document.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::OfficeDocumentEdit { .. }
                    ));
                }
                "office.sheet.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::OfficeSheetEdit { .. }
                    ));
                }
                "office.slides.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::OfficeSlidesEdit { .. }
                    ));
                }
                "miniapp.read" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppRead { .. }
                    ));
                }
                "miniapp.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppEdit { .. }
                    ));
                }
                "miniapp.publish" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppPublish { .. }
                    ));
                }
                "miniapp.serve" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppServe { .. }
                    ));
                }
                other => panic!("unexpected Wave 3 capability {other}"),
            }

            validate_resource_bindings(
                &capability_id,
                "wave3-test-owner",
                capability.requirements,
                &bindings,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} resource requirements no longer fit canonical bindings: {error}",
                    capability.id
                )
            });
        }
    }

    #[test]
    fn unconfigured_action_host_returns_a_typed_unavailable_error() {
        let host_port = unconfigured_host_port();
        let future = host_port.invoke(Wave3HostRequest {
            context: Wave3HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "wave3-test-owner".to_owned(),
                },
                agent_session_id: AgentSessionId::from("wave3-test-session"),
                operation_id: OperationId::from("wave3-test-operation"),
                idempotency_key: IdempotencyKey::from("wave3-test-idempotency"),
                correlation_id: CorrelationId::from("wave3-test-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from("creation.text"),
                action_id: ActionId::from("creation.text"),
                state_scope_key: ScopeKey::from("session:wave3-test"),
                resource_bindings: Vec::new(),
            },
            operation: Wave3CapabilityOperation::CreationText {
                input: StrictJsonValue(serde_json::json!({})),
            },
        });
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        let result = match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => result.expect_err("unconfigured Wave 3 actions must fail closed"),
            Poll::Pending => panic!("unconfigured Wave 3 adapter must fail immediately"),
        };
        assert_eq!(result.code, "WAVE3_HOST_PORT_UNAVAILABLE");
        assert_eq!(
            result.message,
            "no production host adapter is bound for creation.text"
        );
    }
}
