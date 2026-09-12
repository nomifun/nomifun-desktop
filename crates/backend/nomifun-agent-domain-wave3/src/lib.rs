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
    CapabilityActionDescriptor, CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind, CapabilityManifest, ConnectionConfigRef, CorrelationId, EffectClass,
    HostPortBindingDescriptor, HostPortId, HostPortRef, IdempotencyKey,
    InProcessEntrypointMetadata, LocalizedMetadata,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginContextDescriptor,
    PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor, PluginMountId,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginRegistrationMetadata,
    PluginSourceKind, PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    SkillId, StrictJsonValue, ToolPresentationKind, TypedResourceBinding, TypedResourceBindings,
    ValidatedPluginConfig, VersionString, capability_surface_declarations,
    digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde::Deserialize;
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
pub const CREATIVE_ASSET_LIBRARY_RESOURCE_ID: &str = "creative-studio-assets";
pub const GENERATION_PROVIDER_RESOURCE_KIND: &str = "generation_provider";
/// Model selector frozen with a `generation_provider` resource binding. The
/// resource ID is the canonical provider ID; the selected model is data of
/// that exact provider resource rather than free-form action input.
pub const GENERATION_PROVIDER_MODEL_PARAMETER: &str = "model";
pub const GENERATION_PROVIDER_MODEL_PARAMETER_PREFIX: &str = "model.";
pub const MINIAPP_RESOURCE_KIND: &str = "miniapp";

pub const TARGET_PACKAGE_IDS: [&str; 4] = [
    CREATION_PACKAGE_ID,
    WORKSHOP_PACKAGE_ID,
    OFFICE_PACKAGE_ID,
    MINIAPP_PACKAGE_ID,
];

pub const TARGET_CAPABILITY_IDS: [&str; 18] = [
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
pub const ALL_CAPABILITY_IDS: [&str; 18] = TARGET_CAPABILITY_IDS;
pub const AGENT_SURFACES: &[&str] = &["desktop", "headless", "remote", "web"];

/// The single host port for action-bearing Wave 3 capabilities.
///
/// The domain crate owns the capability vocabulary and resource requirements.
/// The application owns creation, Canvas, Office, and MiniApp facts and must
/// provide the adapter used by [`registrations_with_host_port`].
pub const WAVE3_CAPABILITY_HOST_PORT_ID: &str = "host.wave3.capability.invoke";
pub const WAVE3_HOST_PORT_UNAVAILABLE: &str = "WAVE3_HOST_PORT_UNAVAILABLE";
pub const WAVE3_INVALID_REQUEST: &str = "WAVE3_INVALID_REQUEST";
pub const WAVE3_ACTION_OPERATION_MISMATCH: &str = "WAVE3_ACTION_OPERATION_MISMATCH";
pub const WAVE3_RESOURCE_BINDING_INVALID: &str = "WAVE3_RESOURCE_BINDING_INVALID";
pub const WAVE3_RESOURCE_OWNER_MISMATCH: &str = "RESOURCE_OWNER_MISMATCH";
pub const WAVE3_RESOURCE_NOT_BOUND: &str = "WAVE3_RESOURCE_NOT_BOUND";
pub const WAVE3_INVALID_RESPONSE: &str = "WAVE3_INVALID_RESPONSE";

const MAX_PROMPT_CHARS: usize = 65_536;
const MAX_SYSTEM_CHARS: usize = 65_536;
const MAX_CREATION_RESULTS: usize = 10;
const MAX_IMAGE_EDIT_INPUTS: usize = 8;

/// The durable Creative Studio aggregate that owns a generated task.
///
/// This is deliberately not inferred by the host. Creation tasks participate
/// in Canvas/template/workbench cleanup and reconciliation, so every caller
/// must name the exact aggregate used by `CreationService`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreationTaskTarget {
    CanvasNode {
        canvas_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        workbench_kind: CreationWorkbenchKind,
    },
    TemplateStep {
        template_id: String,
        template_run_id: String,
        template_step_id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreationWorkbenchKind {
    Image,
    Video,
    Audio,
}

impl CreationWorkbenchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreationImageInputRole {
    Reference,
    Mask,
}

impl CreationImageInputRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Mask => "mask",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationImageInput {
    pub asset_id: String,
    pub role: CreationImageInputRole,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationTextRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub system: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationImageRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    #[serde(default = "default_creation_count")]
    pub count: u32,
    pub size: Option<String>,
    pub quality: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationImageEditRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub inputs: Vec<CreationImageInput>,
    #[serde(default = "default_creation_count")]
    pub count: u32,
    pub size: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationVideoRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub seconds: Option<u32>,
    pub size: Option<String>,
    pub first_frame_asset_id: Option<String>,
    pub last_frame_asset_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreationAudioRequest {
    pub target: CreationTaskTarget,
    pub text: String,
    pub voice: Option<String>,
    pub format: Option<String>,
}

const fn default_max_tokens() -> u32 {
    4_096
}

const fn default_creation_count() -> u32 {
    1
}

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
/// No application service bag, Gateway state, legacy Conversation state,
/// `PluginStateHandle`, or other Kernel authority is exposed through this
/// boundary. The central adapter owns its real business persistence (for
/// example, an injected Creation/Workshop/Office/MiniApp service or
/// repository) and uses this context's principal, snapshot, idempotency,
/// correlation, and resource identities to authorize and persist the action.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationProviderSelection {
    pub provider_id: String,
    pub model: String,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision: i64,
}

/// Resolve the exact provider/model selected by the frozen resource binding.
///
/// Domain adapters must not accept provider identity from action input. This
/// keeps retries on the same configured resource and prevents a model from
/// escaping the Snapshot's authority by changing a payload field.
pub fn generation_provider_selection(
    context: &Wave3HostContext,
) -> Result<GenerationProviderSelection, Wave3HostPortError> {
    let bindings = context
        .resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == GENERATION_PROVIDER_RESOURCE_KIND)
        .collect::<Vec<_>>();
    let [binding] = bindings.as_slice() else {
        return Err(Wave3HostPortError::resource_binding_invalid(
            "creation requires exactly one generation_provider resource binding",
        ));
    };
    let accepted_capability_keys = [
        "creation.text",
        "creation.image",
        "creation.image_edit",
        "creation.video",
        "creation.audio",
    ]
    .map(|capability_id| format!("{GENERATION_PROVIDER_MODEL_PARAMETER_PREFIX}{capability_id}"));
    if binding.typed_parameters.keys().any(|key| {
        key != GENERATION_PROVIDER_MODEL_PARAMETER
            && !accepted_capability_keys.iter().any(|accepted| accepted == key)
    }) {
        return Err(Wave3HostPortError::resource_binding_invalid(
            "generation_provider accepts only model or model.creation.* typed parameters",
        ));
    }
    let capability_model_key = format!(
        "{GENERATION_PROVIDER_MODEL_PARAMETER_PREFIX}{}",
        context.capability_id.as_ref()
    );
    let model = binding
        .typed_parameters
        .get(&capability_model_key)
        .or_else(|| {
            binding
                .typed_parameters
                .get(GENERATION_PROVIDER_MODEL_PARAMETER)
        })
        .ok_or_else(|| {
            Wave3HostPortError::resource_binding_invalid(
                format!(
                    "generation_provider requires {capability_model_key} or model"
                ),
            )
        })?;
    if model.trim().is_empty() || model.trim() != model {
        return Err(Wave3HostPortError::resource_binding_invalid(
            "generation_provider model must be non-empty and already trimmed",
        ));
    }
    let connection_config_ref = binding.connection_config_ref.clone().ok_or_else(|| {
        Wave3HostPortError::resource_binding_invalid(
            "generation_provider requires a frozen connection_config_ref",
        )
    })?;
    let expected_prefix = format!("provider:{}@", binding.resource_id.as_ref());
    let revision = connection_config_ref
        .as_ref()
        .strip_prefix(&expected_prefix)
        .ok_or_else(|| {
            Wave3HostPortError::resource_binding_invalid(
                "generation_provider connection_config_ref does not match its provider resource",
            )
        })?;
    let config_revision = revision.parse::<i64>().map_err(|_| {
        Wave3HostPortError::resource_binding_invalid(
            "generation_provider connection_config_ref requires a canonical non-negative revision",
        )
    })?;
    if config_revision < 0 || config_revision.to_string() != revision {
        return Err(Wave3HostPortError::resource_binding_invalid(
            "generation_provider connection_config_ref requires a canonical non-negative revision",
        ));
    }
    Ok(GenerationProviderSelection {
        provider_id: binding.resource_id.as_ref().to_owned(),
        model: model.to_owned(),
        connection_config_ref,
        config_revision,
    })
}

/// Typed domain-family operations accepted by the Wave 3 host.
///
/// Payload schemas remain owned by each capability.  The enum prevents the
/// registration crate from fabricating a result while allowing the owning
/// domain to validate and interpret its input.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave3CapabilityOperation {
    CreationText(CreationTextRequest),
    CreationImage(CreationImageRequest),
    CreationImageEdit(CreationImageEditRequest),
    CreationVideo(CreationVideoRequest),
    CreationAudio(CreationAudioRequest),
    WorkshopCanvasRead { input: StrictJsonValue },
    WorkshopCanvasEdit { input: StrictJsonValue },
    WorkshopAssetRead { input: StrictJsonValue },
    WorkshopAssetWrite { input: StrictJsonValue },
    WorkshopTemplateRun { input: StrictJsonValue },
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

impl Wave3CapabilityOperation {
    /// Return the canonical capability identity fixed by this typed variant.
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::CreationText(_) => "creation.text",
            Self::CreationImage(_) => "creation.image",
            Self::CreationImageEdit(_) => "creation.image_edit",
            Self::CreationVideo(_) => "creation.video",
            Self::CreationAudio(_) => "creation.audio",
            Self::WorkshopCanvasRead { .. } => "workshop.canvas.read",
            Self::WorkshopCanvasEdit { .. } => "workshop.canvas.edit",
            Self::WorkshopAssetRead { .. } => "workshop.asset.read",
            Self::WorkshopAssetWrite { .. } => "workshop.asset.write",
            Self::WorkshopTemplateRun { .. } => "workshop.template.run",
            Self::OfficePreview { .. } => "office.preview",
            Self::OfficeDocumentEdit { .. } => "office.document.edit",
            Self::OfficeSheetEdit { .. } => "office.sheet.edit",
            Self::OfficeSlidesEdit { .. } => "office.slides.edit",
            Self::MiniAppRead { .. } => "miniapp.read",
            Self::MiniAppEdit { .. } => "miniapp.edit",
            Self::MiniAppPublish { .. } => "miniapp.publish",
            Self::MiniAppServe { .. } => "miniapp.serve",
        })
    }

    /// Return the canonical action identity paired with this operation.
    pub fn action_id(&self) -> ActionId {
        action_id(self.capability_id().as_ref())
            .expect("every Wave 3 operation is action-bearing")
    }

    /// Return the first-party owner domain for the operation.
    pub fn owner_domain(&self) -> Wave3OwnerDomain {
        match self {
            Self::CreationText(_)
            | Self::CreationImage(_)
            | Self::CreationImageEdit(_)
            | Self::CreationVideo(_)
            | Self::CreationAudio(_) => Wave3OwnerDomain::Creation,
            Self::WorkshopCanvasRead { .. }
            | Self::WorkshopCanvasEdit { .. }
            | Self::WorkshopAssetRead { .. }
            | Self::WorkshopAssetWrite { .. }
            | Self::WorkshopTemplateRun { .. } => Wave3OwnerDomain::Workshop,
            Self::OfficePreview { .. }
            | Self::OfficeDocumentEdit { .. }
            | Self::OfficeSheetEdit { .. }
            | Self::OfficeSlidesEdit { .. } => Wave3OwnerDomain::Office,
            Self::MiniAppRead { .. }
            | Self::MiniAppEdit { .. }
            | Self::MiniAppPublish { .. }
            | Self::MiniAppServe { .. } => Wave3OwnerDomain::MiniApp,
        }
    }

    pub fn validate(&self) -> Result<(), Wave3HostPortError> {
        match self {
            Self::CreationText(request) => validate_creation_text(request),
            Self::CreationImage(request) => validate_creation_image(request),
            Self::CreationImageEdit(request) => validate_creation_image_edit(request),
            Self::CreationVideo(request) => validate_creation_video(request),
            Self::CreationAudio(request) => validate_creation_audio(request),
            Self::WorkshopCanvasRead { input }
            | Self::WorkshopCanvasEdit { input }
            | Self::WorkshopAssetRead { input }
            | Self::WorkshopAssetWrite { input }
            | Self::WorkshopTemplateRun { input }
            | Self::OfficePreview { input }
            | Self::OfficeDocumentEdit { input }
            | Self::OfficeSheetEdit { input }
            | Self::OfficeSlidesEdit { input }
            | Self::MiniAppRead { input }
            | Self::MiniAppEdit { input }
            | Self::MiniAppPublish { input }
            | Self::MiniAppServe { input } => {
                if input.0.is_object() {
                    Ok(())
                } else {
                    Err(Wave3HostPortError::invalid_request(format!(
                        "{} input must be a JSON object",
                        self.capability_id().as_ref()
                    )))
                }
            }
        }
    }
}

impl Wave3HostRequest {
    /// Validate the complete boundary before an owner receives the request.
    pub fn validate(&self) -> Result<(), Wave3HostPortError> {
        let capability_id = &self.context.capability_id;
        let Some(spec) = find_capability(capability_id.as_ref()) else {
            return Err(Wave3HostPortError::invalid_request(format!(
                "unknown Wave 3 capability {}",
                capability_id.as_ref()
            )));
        };
        let operation_capability_id = self.operation.capability_id();
        let operation_action_id = self.operation.action_id();
        if operation_capability_id != *capability_id
            || operation_action_id != self.context.action_id
        {
            return Err(Wave3HostPortError::action_operation_mismatch(format!(
                "context maps {} / {} but typed operation maps {} / {}",
                capability_id.as_ref(),
                self.context.action_id.as_ref(),
                operation_capability_id.as_ref(),
                operation_action_id.as_ref()
            )));
        }
        self.operation.validate()?;
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
        Self::new(WAVE3_HOST_PORT_UNAVAILABLE, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(WAVE3_INVALID_REQUEST, message)
    }

    pub fn action_operation_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE3_ACTION_OPERATION_MISMATCH, message)
    }

    pub fn resource_binding_invalid(message: impl Into<String>) -> Self {
        Self::new(WAVE3_RESOURCE_BINDING_INVALID, message)
    }

    pub fn resource_owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(WAVE3_RESOURCE_OWNER_MISMATCH, message)
    }

    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self::new(WAVE3_INVALID_RESPONSE, message)
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
            request.validate()?;
            Err(Wave3HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

/// The owner domains that may be injected independently by central
/// composition. Each owner still receives the same validated typed request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave3OwnerDomain {
    Creation,
    Workshop,
    Office,
    MiniApp,
}

/// Optional first-party owner bindings for the canonical Wave 3 action port.
///
/// Central composition can connect only owners backed by real services.
/// Missing owners remain unavailable; this type never supplies a success
/// fallback or a synthetic result.
#[derive(Default)]
pub struct Wave3OwnerBindings {
    pub creation: Option<Arc<dyn Wave3HostPort>>,
    pub workshop: Option<Arc<dyn Wave3HostPort>>,
    pub office: Option<Arc<dyn Wave3HostPort>>,
    pub miniapp: Option<Arc<dyn Wave3HostPort>>,
}

impl Wave3OwnerBindings {
    pub fn with_creation(mut self, owner: Arc<dyn Wave3HostPort>) -> Self {
        self.creation = Some(owner);
        self
    }

    pub fn with_workshop(mut self, owner: Arc<dyn Wave3HostPort>) -> Self {
        self.workshop = Some(owner);
        self
    }

    pub fn with_office(mut self, owner: Arc<dyn Wave3HostPort>) -> Self {
        self.office = Some(owner);
        self
    }

    pub fn with_miniapp(mut self, owner: Arc<dyn Wave3HostPort>) -> Self {
        self.miniapp = Some(owner);
        self
    }
}

/// Compose independently injected owners behind the one manifest host port.
pub fn composed_host_port(bindings: Wave3OwnerBindings) -> Arc<dyn Wave3HostPort> {
    Arc::new(ComposedWave3HostPort { bindings })
}

struct ComposedWave3HostPort {
    bindings: Wave3OwnerBindings,
}

impl Wave3HostPort for ComposedWave3HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        if let Err(error) = request.validate() {
            return Box::pin(async move { Err(error) });
        }

        let owner = match request.operation.owner_domain() {
            Wave3OwnerDomain::Creation => self.bindings.creation.clone(),
            Wave3OwnerDomain::Workshop => self.bindings.workshop.clone(),
            Wave3OwnerDomain::Office => self.bindings.office.clone(),
            Wave3OwnerDomain::MiniApp => self.bindings.miniapp.clone(),
        };
        let capability_id = request.context.capability_id.clone();
        Box::pin(async move {
            let Some(owner) = owner else {
                return Err(Wave3HostPortError::unavailable(format!(
                    "no production owner is bound for {}",
                    capability_id.as_ref()
                )));
            };
            owner.invoke(request).await
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

const WORKSHOP_CAPABILITIES: [CapabilitySpec; 5] = [
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
        effect_class: EffectClass::ReadSensitive,
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
        description: "Bundled Canvas, asset, and template capabilities.",
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

/// Return the operations exposed by each canonical Wave 3 resource kind.
pub fn resource_binding_metadata() -> BTreeMap<ResourceKind, BTreeSet<String>> {
    typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect()
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

/// Return the canonical action identity for an action-bearing capability.
pub fn action_id(capability_id: &str) -> Option<ActionId> {
    find_capability(capability_id).map(|_| ActionId::from(format!("{capability_id}.invoke")))
}

fn validate_capability_specs(spec: &PackageSpec) -> Result<(), String> {
    let resource_metadata = resource_binding_metadata();
    let mut capability_ids = BTreeSet::new();

    for capability in spec.capabilities {
        if !capability_ids.insert(capability.id) {
            return Err(format!(
                "duplicate Wave 3 capability {} in package {}",
                capability.id, spec.id
            ));
        }

        let declared_kinds = capability
            .resource_kinds
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let required_kinds = capability
            .requirements
            .iter()
            .map(|requirement| requirement.resource_kind)
            .collect::<BTreeSet<_>>();
        if declared_kinds != required_kinds {
            return Err(format!(
                "Wave 3 capability {} resource kinds do not match its requirements",
                capability.id
            ));
        }

        let mut requirement_keys = BTreeSet::new();
        for requirement in capability.requirements {
            if !requirement_keys.insert((requirement.resource_kind, requirement.operation)) {
                return Err(format!(
                    "Wave 3 capability {} declares duplicate resource requirement {}:{}",
                    capability.id, requirement.resource_kind, requirement.operation
                ));
            }
            let Some(operations) =
                resource_metadata.get(&ResourceKind::from(requirement.resource_kind))
            else {
                return Err(format!(
                    "Wave 3 capability {} requires unknown resource kind {}",
                    capability.id, requirement.resource_kind
                ));
            };
            if !operations.contains(requirement.operation) {
                return Err(format!(
                    "Wave 3 capability {} requires unsupported operation {} on {}",
                    capability.id, requirement.operation, requirement.resource_kind
                ));
            }
        }
    }
    Ok(())
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
    validate_capability_specs(spec)?;
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
            declared_role_ids: BTreeSet::new(),
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
                    action_id: action_id(capability.id)
                        .expect("every Wave 3 capability is action-bearing"),
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
    let input_schema = action_input_schema_for(spec.id)?.0;
    let output_schema = action_output_schema_for(spec.id)?.0;
    let input_digest = digest_payload(&input_schema).map_err(|error| error.to_string())?;
    let output_digest = digest_payload(&output_schema).map_err(|error| error.to_string())?;
    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "capability:{}",
            spec.id
        )),
        version: VersionString::from(PACKAGE_VERSION),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: capability_display(spec),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            AGENT_SURFACES.iter().copied(),
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: capability_config_schema(),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: action_id(spec.id)
                    .expect("every Wave 3 capability is action-bearing"),
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

/// Return the canonical input schema embedded in one Wave 3 action reference.
///
/// Application adapters use this same resolver as manifest generation, so a
/// typed implementation cannot silently drift from the schema advertised to
/// models and other consumers.
pub fn action_input_schema_for(capability_id: &str) -> Result<StrictJsonValue, String> {
    if find_capability(capability_id).is_none() {
        return Err(format!("unknown Wave 3 capability {capability_id}"));
    }
    Ok(StrictJsonValue(match capability_id {
        "creation.text" => strict_object_schema(
            json!({
                "target": creation_target_schema(None),
                "prompt": bounded_string_schema(MAX_PROMPT_CHARS),
                "system": {"type": "string", "maxLength": MAX_SYSTEM_CHARS},
                "max_tokens": {"type": "integer", "minimum": 1, "maximum": 131072}
            }),
            &["target", "prompt"],
        ),
        "creation.image" => strict_object_schema(
            json!({
                "target": creation_target_schema(Some("image")),
                "prompt": bounded_string_schema(MAX_PROMPT_CHARS),
                "count": {"type": "integer", "minimum": 1, "maximum": MAX_CREATION_RESULTS},
                "size": bounded_string_schema(128),
                "quality": bounded_string_schema(128)
            }),
            &["target", "prompt"],
        ),
        "creation.image_edit" => strict_object_schema(
            json!({
                "target": creation_target_schema(Some("image")),
                "prompt": bounded_string_schema(MAX_PROMPT_CHARS),
                "inputs": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_IMAGE_EDIT_INPUTS,
                    "items": strict_object_schema(
                        json!({
                            "asset_id": uuidv7_schema(),
                            "role": {"enum": ["reference", "mask"]}
                        }),
                        &["asset_id", "role"],
                    )
                },
                "count": {"type": "integer", "minimum": 1, "maximum": MAX_CREATION_RESULTS},
                "size": bounded_string_schema(128)
            }),
            &["target", "prompt", "inputs"],
        ),
        "creation.video" => {
            let mut schema = strict_object_schema(
                json!({
                "target": creation_target_schema(Some("video")),
                "prompt": bounded_string_schema(MAX_PROMPT_CHARS),
                "seconds": {"type": "integer", "minimum": 1, "maximum": 3600},
                "size": bounded_string_schema(128),
                "first_frame_asset_id": uuidv7_schema(),
                "last_frame_asset_id": uuidv7_schema()
                }),
                &["target", "prompt"],
            );
            schema["dependentRequired"] = json!({
                "last_frame_asset_id": ["first_frame_asset_id"]
            });
            schema
        }
        "creation.audio" => strict_object_schema(
            json!({
                "target": creation_target_schema(Some("audio")),
                "text": bounded_string_schema(MAX_PROMPT_CHARS),
                "voice": bounded_string_schema(256),
                "format": bounded_string_schema(64)
            }),
            &["target", "text"],
        ),
        "workshop.canvas.read" => strict_object_schema(json!({}), &[]),
        "workshop.canvas.edit" => strict_object_schema(
            json!({
                "expected_revision": revision_schema(),
                "document": canvas_document_schema()
            }),
            &["expected_revision", "document"],
        ),
        "workshop.asset.read" => strict_object_schema(
            json!({"asset_id": uuidv7_schema()}),
            &["asset_id"],
        ),
        "workshop.asset.write" => strict_object_schema(
            json!({
                "asset_id": uuidv7_schema(),
                "title": bounded_string_schema(1_000),
                "collection": {"type": "string", "maxLength": 1_000},
                "tags": {
                    "type": "array",
                    "maxItems": 100,
                    "uniqueItems": true,
                    "items": bounded_string_schema(120)
                },
                "in_library": {"type": "boolean"}
            }),
            &["asset_id"],
        ),
        "workshop.template.run" => strict_object_schema(
            json!({
                "templateId": uuidv7_schema(),
                "templateRevision": {"type": "integer", "minimum": 1},
                "inputs": {"type": "array", "maxItems": 100, "items": {"type": "object"}},
                "referenceAssetIds": {
                    "type": "array",
                    "maxItems": 100,
                    "uniqueItems": true,
                    "items": uuidv7_schema()
                }
            }),
            &["templateId", "templateRevision", "inputs", "referenceAssetIds"],
        ),
        "miniapp.read" => strict_object_schema(
            json!({"path": bounded_string_schema(4_096)}),
            &[],
        ),
        "miniapp.edit" => strict_object_schema(
            json!({
                "expected_product_revision": {"type": "integer", "minimum": 0},
                "project_id": uuidv7_schema(),
                "expected_project_revision": {"type": "integer", "minimum": 0},
                "expected_build_generation": {"type": "integer", "minimum": 0},
                "expected_source_snapshot_digest": digest_schema(),
                "path": bounded_string_schema(4_096),
                "content": {"type": "string", "maxLength": 4_194_304}
            }),
            &[
                "expected_product_revision",
                "project_id",
                "expected_project_revision",
                "expected_build_generation",
                "expected_source_snapshot_digest",
                "path",
                "content",
            ],
        ),
        "miniapp.publish" => strict_object_schema(
            json!({
                "expected_product_revision": {"type": "integer", "minimum": 0},
                "expected_pointer_revision": {"type": "integer", "minimum": 0},
                "expected_active_release_epoch": {"type": "integer", "minimum": 0},
                "ready_release_id": uuidv7_schema(),
                "expected_ready_release_digest": digest_schema(),
                "expected_active_release_digest": digest_schema(),
                "expected_service_test_receipt_id": uuidv7_schema(),
                "acknowledge_test_warning": {"type": "boolean"}
            }),
            &[
                "expected_product_revision",
                "expected_pointer_revision",
                "expected_active_release_epoch",
                "ready_release_id",
                "expected_ready_release_digest",
                "acknowledge_test_warning",
            ],
        ),
        "miniapp.serve" => strict_object_schema(json!({}), &[]),
        _ => json!({
            "type": "object",
            "additionalProperties": true
        }),
    }))
}

/// Return the canonical output schema embedded in one Wave 3 action reference.
pub fn action_output_schema_for(capability_id: &str) -> Result<StrictJsonValue, String> {
    if find_capability(capability_id).is_none() {
        return Err(format!("unknown Wave 3 capability {capability_id}"));
    }
    Ok(StrictJsonValue(match capability_id {
        "creation.text"
        | "creation.image"
        | "creation.image_edit"
        | "creation.video"
        | "creation.audio" => strict_object_schema(
            json!({
                "creation_task_id": uuidv7_schema(),
                "status": {"const": "succeeded"},
                "result_asset_ids": {
                    "type": "array",
                    "maxItems": MAX_CREATION_RESULTS,
                    "uniqueItems": true,
                    "items": uuidv7_schema()
                }
            }),
            &["creation_task_id", "status", "result_asset_ids"],
        ),
        "workshop.canvas.read" => strict_object_schema(
            json!({
                "canvas": canvas_summary_schema(),
                "document": canvas_document_schema()
            }),
            &["canvas", "document"],
        ),
        "workshop.canvas.edit" => canvas_summary_schema(),
        "workshop.asset.read" | "workshop.asset.write" => workshop_asset_schema(),
        "workshop.template.run" => strict_object_schema(
            json!({
                "kind": {"const": "nomifun.creative-studio.template-run"},
                "version": {"const": 1},
                "revision": {"type": "integer", "minimum": 1},
                "templateSnapshot": {"type": "object"},
                "request": {"type": "object"},
                "promptDrafts": {"type": "array"},
                "record": strict_object_schema(
                    json!({
                        "requestId": uuidv7_schema(),
                        "templateId": uuidv7_schema(),
                        "status": {"const": "succeeded"},
                        "promptDraftIds": {"type": "array", "items": uuidv7_schema()},
                        "taskIds": {"type": "array", "items": uuidv7_schema()},
                        "resultAssetIds": {"type": "array", "items": uuidv7_schema()},
                        "historyReferenceIds": {"type": "array", "items": uuidv7_schema()},
                        "queuedAt": {"type": "integer", "minimum": 0},
                        "startedAt": {"type": "integer", "minimum": 0},
                        "completedAt": {"type": "integer", "minimum": 0},
                        "failure": {"type": "null"}
                    }),
                    &[
                        "requestId", "templateId", "status", "promptDraftIds", "taskIds",
                        "resultAssetIds", "historyReferenceIds", "queuedAt", "startedAt",
                        "completedAt", "failure",
                    ],
                )
            }),
            &["kind", "version", "revision", "templateSnapshot", "request", "promptDrafts", "record"],
        ),
        "miniapp.read" => json!({
            "oneOf": [miniapp_workshop_schema(), miniapp_source_file_schema()]
        }),
        "miniapp.edit" | "miniapp.publish" => miniapp_workshop_schema(),
        "miniapp.serve" => miniapp_workshop_schema(),
        _ => json!({
            "type": "object",
            "additionalProperties": true
        }),
    }))
}

/// Resolve a manifest schema reference only when its subject, role and digest
/// exactly match the current canonical Wave 3 action contract.
pub fn resolve_action_schema(
    capability_id: &str,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    for (role, schema) in [
        ("input", action_input_schema_for(capability_id)?),
        ("output", action_output_schema_for(capability_id)?),
    ] {
        let expected = schema_ref(capability_id, role, &schema.0)?;
        if expected == *reference {
            return Ok(schema);
        }
    }
    Err(format!(
        "schema reference {} does not match {capability_id}'s canonical input or output schema",
        reference.as_ref()
    ))
}

fn strict_object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required
    })
}

fn bounded_string_schema(max_length: usize) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max_length})
}

fn uuidv7_schema() -> Value {
    json!({
        "type": "string",
        "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
    })
}

fn digest_schema() -> Value {
    json!({"type": "string", "pattern": "^[0-9a-f]{64}$"})
}

fn revision_schema() -> Value {
    json!({"type": "string", "pattern": "^(0|[1-9][0-9]*)$"})
}

fn canvas_summary_schema() -> Value {
    strict_object_schema(
        json!({
            "canvasId": uuidv7_schema(),
            "title": {"type": "string"},
            "revision": revision_schema(),
            "nodeCount": {"type": "integer", "minimum": 0},
            "connectionCount": {"type": "integer", "minimum": 0},
            "createdAt": {"type": "integer", "minimum": 0},
            "updatedAt": {"type": "integer", "minimum": 0}
        }),
        &["canvasId", "title", "revision", "nodeCount", "connectionCount", "createdAt", "updatedAt"],
    )
}

fn canvas_document_schema() -> Value {
    strict_object_schema(
        json!({
            "schema": {"type": "string"},
            "canvasId": uuidv7_schema(),
            "viewport": {"type": "object"},
            "background": {"type": "object"},
            "nodes": {"type": "array"},
            "connections": {"type": "array"},
            "chatSessions": {"type": "array"},
            "activeChatId": {"type": ["string", "null"]},
            "panels": {"type": "object"},
            "pendingTaskIds": {"type": "array", "items": uuidv7_schema()}
        }),
        &[
            "schema", "canvasId", "viewport", "background", "nodes", "connections",
            "chatSessions", "activeChatId", "panels", "pendingTaskIds",
        ],
    )
}

fn workshop_asset_schema() -> Value {
    strict_object_schema(
        json!({
            "asset_id": uuidv7_schema(),
            "kind": {"type": "string"},
            "title": {"type": "string"},
            "collection": {"type": ["string", "null"]},
            "tags": {"type": "array", "items": {"type": "string"}},
            "mime": {"type": ["string", "null"]},
            "width": {"type": ["integer", "null"]},
            "height": {"type": ["integer", "null"]},
            "bytes": {"type": ["integer", "null"]},
            "in_library": {"type": "boolean"},
            "deleted_at": {"type": ["integer", "null"]},
            "text_content": {"type": ["string", "null"]},
            "origin": {},
            "url": {"type": "string"},
            "thumb_url": {"type": ["string", "null"]},
            "created_at": {"type": "integer"},
            "updated_at": {"type": "integer"}
        }),
        &[
            "asset_id", "kind", "title", "collection", "tags", "mime", "width", "height",
            "bytes", "in_library", "deleted_at", "text_content", "origin", "url",
            "thumb_url", "created_at", "updated_at",
        ],
    )
}

fn miniapp_workshop_schema() -> Value {
    strict_object_schema(
        json!({
            "miniapp": {"type": "object"},
            "service_lifecycle": {"type": "object"},
            "active_service": {"type": "object"},
            "publish_mode": {"type": "string"},
            "project_id": uuidv7_schema(),
            "project_revision": {"type": "integer", "minimum": 0},
            "source_state": {"type": "string"},
            "build_generation": {"type": "integer", "minimum": 0},
            "source_snapshot_digest": digest_schema(),
            "dependency_lock_digest": digest_schema(),
            "ready": {"type": "object"},
            "config_schema": {"type": "object"},
            "config": {"type": "object"},
            "credential_bindings_revision": {"type": "integer", "minimum": 0},
            "credential_slots": {"type": "array"},
            "capabilities": {"type": "array"},
            "active_operation": {"type": "object"}
        }),
        &[
            "miniapp", "publish_mode", "project_id", "project_revision", "source_state",
            "build_generation", "config_schema", "config", "credential_bindings_revision",
            "credential_slots", "capabilities",
        ],
    )
}

fn miniapp_source_file_schema() -> Value {
    strict_object_schema(
        json!({
            "miniapp_id": uuidv7_schema(),
            "project_id": uuidv7_schema(),
            "path": {"type": "string"},
            "content": {"type": "string"},
            "source_snapshot_digest": digest_schema(),
            "build_generation": {"type": "integer", "minimum": 0}
        }),
        &["miniapp_id", "project_id", "path", "content", "source_snapshot_digest", "build_generation"],
    )
}

fn creation_target_schema(standalone_kind: Option<&str>) -> Value {
    let mut targets = vec![
        strict_object_schema(
            json!({
                "kind": {"const": "canvas_node"},
                "canvas_id": uuidv7_schema(),
                "node_id": uuidv7_schema()
            }),
            &["kind", "canvas_id", "node_id"],
        ),
        strict_object_schema(
            json!({
                "kind": {"const": "template_step"},
                "template_id": uuidv7_schema(),
                "template_run_id": uuidv7_schema(),
                "template_step_id": uuidv7_schema()
            }),
            &["kind", "template_id", "template_run_id", "template_step_id"],
        ),
    ];
    if let Some(workbench_kind) = standalone_kind {
        targets.push(strict_object_schema(
            json!({
                "kind": {"const": "standalone_workbench"},
                "workbench_kind": {"const": workbench_kind}
            }),
            &["kind", "workbench_kind"],
        ));
    }
    json!({"oneOf": targets})
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
    let request_schema = json!({
        "anyOf": ALL_CAPABILITY_IDS
            .iter()
            .map(|capability_id| action_input_schema_for(capability_id).map(|schema| schema.0))
            .collect::<Result<Vec<_>, _>>()?
    });
    let response_schema = json!({
        "anyOf": ALL_CAPABILITY_IDS
            .iter()
            .map(|capability_id| action_output_schema_for(capability_id).map(|schema| schema.0))
            .collect::<Result<Vec<_>, _>>()?
    });
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
            let request = Wave3HostRequest {
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
            };
            request
                .validate()
                .map_err(wave3_host_error_to_kernel)?;
            let result = self
                .host_port
                .invoke(request)
                .await
                .map_err(wave3_host_error_to_kernel)?;
            if !result.0.is_object() {
                return Err(wave3_host_error_to_kernel(
                    Wave3HostPortError::invalid_response(format!(
                        "{} host result must be a JSON object",
                        self.capability_id.as_ref()
                    )),
                ));
            }
            Ok(result)
        })
    }
}

fn wave3_host_error_to_kernel(error: Wave3HostPortError) -> KernelError {
    KernelError::capability_execution_failed(error.code, error.message)
}

/// Convert a canonical capability ID and its object payload into the only
/// typed operation variant accepted by the host port.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave3CapabilityOperation, KernelError> {
    let operation = match capability_id.as_ref() {
        "creation.text" => Wave3CapabilityOperation::CreationText(
            parse_creation_request(input.0, "creation.text")?,
        ),
        "creation.image" => Wave3CapabilityOperation::CreationImage(
            parse_creation_request(input.0, "creation.image")?,
        ),
        "creation.image_edit" => Wave3CapabilityOperation::CreationImageEdit(
            parse_creation_request(input.0, "creation.image_edit")?,
        ),
        "creation.video" => Wave3CapabilityOperation::CreationVideo(
            parse_creation_request(input.0, "creation.video")?,
        ),
        "creation.audio" => Wave3CapabilityOperation::CreationAudio(
            parse_creation_request(input.0, "creation.audio")?,
        ),
        "workshop.canvas.read" => Wave3CapabilityOperation::WorkshopCanvasRead { input },
        "workshop.canvas.edit" => Wave3CapabilityOperation::WorkshopCanvasEdit { input },
        "workshop.asset.read" => Wave3CapabilityOperation::WorkshopAssetRead { input },
        "workshop.asset.write" => Wave3CapabilityOperation::WorkshopAssetWrite { input },
        "workshop.template.run" => Wave3CapabilityOperation::WorkshopTemplateRun { input },
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
    operation
        .validate()
        .map_err(wave3_host_error_to_kernel)?;
    Ok(operation)
}

fn parse_creation_request<T: for<'de> Deserialize<'de>>(
    input: Value,
    capability_id: &str,
) -> Result<T, KernelError> {
    serde_json::from_value(input).map_err(|error| {
        wave3_host_error_to_kernel(Wave3HostPortError::invalid_request(format!(
            "invalid {capability_id} input: {error}"
        )))
    })
}

fn validate_creation_text(request: &CreationTextRequest) -> Result<(), Wave3HostPortError> {
    validate_creation_target(&request.target, None)
        .and_then(|_| require_bounded_text("prompt", &request.prompt, MAX_PROMPT_CHARS, false))
        .and_then(|_| {
            request.system.as_deref().map_or(Ok(()), |system| {
                require_bounded_text("system", system, MAX_SYSTEM_CHARS, true)
            })
        })
        .and_then(|_| {
            if (1..=131_072).contains(&request.max_tokens) {
                Ok(())
            } else {
                Err("max_tokens must be between 1 and 131072".to_owned())
            }
        })
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_creation_image(request: &CreationImageRequest) -> Result<(), Wave3HostPortError> {
    validate_creation_target(&request.target, Some(CreationWorkbenchKind::Image))
        .and_then(|_| require_bounded_text("prompt", &request.prompt, MAX_PROMPT_CHARS, false))
        .and_then(|_| validate_creation_count(request.count))
        .and_then(|_| validate_optional_short("size", request.size.as_deref(), 128))
        .and_then(|_| validate_optional_short("quality", request.quality.as_deref(), 128))
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_creation_image_edit(
    request: &CreationImageEditRequest,
) -> Result<(), Wave3HostPortError> {
    validate_creation_target(&request.target, Some(CreationWorkbenchKind::Image))
        .and_then(|_| require_bounded_text("prompt", &request.prompt, MAX_PROMPT_CHARS, false))
        .and_then(|_| validate_creation_count(request.count))
        .and_then(|_| validate_optional_short("size", request.size.as_deref(), 128))
        .and_then(|_| {
            if request.inputs.is_empty() || request.inputs.len() > MAX_IMAGE_EDIT_INPUTS {
                return Err(format!(
                    "inputs must contain 1 to {MAX_IMAGE_EDIT_INPUTS} image references"
                ));
            }
            let mut asset_ids = BTreeSet::new();
            let mut masks = 0;
            for input in &request.inputs {
                require_uuidv7("inputs[].asset_id", &input.asset_id)?;
                if !asset_ids.insert(input.asset_id.as_str()) {
                    return Err(format!("duplicate image input {}", input.asset_id));
                }
                if input.role == CreationImageInputRole::Mask {
                    masks += 1;
                }
            }
            if masks > 1 {
                return Err("inputs may contain at most one mask".to_owned());
            }
            if request
                .inputs
                .iter()
                .all(|input| input.role == CreationImageInputRole::Mask)
            {
                return Err("inputs require at least one reference image".to_owned());
            }
            Ok(())
        })
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_creation_video(request: &CreationVideoRequest) -> Result<(), Wave3HostPortError> {
    validate_creation_target(&request.target, Some(CreationWorkbenchKind::Video))
        .and_then(|_| require_bounded_text("prompt", &request.prompt, MAX_PROMPT_CHARS, false))
        .and_then(|_| {
            if request.seconds.is_some_and(|seconds| seconds == 0 || seconds > 3_600) {
                Err("seconds must be between 1 and 3600".to_owned())
            } else {
                Ok(())
            }
        })
        .and_then(|_| validate_optional_short("size", request.size.as_deref(), 128))
        .and_then(|_| {
            for (label, asset_id) in [
                ("first_frame_asset_id", request.first_frame_asset_id.as_deref()),
                ("last_frame_asset_id", request.last_frame_asset_id.as_deref()),
            ] {
                if let Some(asset_id) = asset_id {
                    require_uuidv7(label, asset_id)?;
                }
            }
            if request.last_frame_asset_id.is_some() && request.first_frame_asset_id.is_none() {
                return Err(
                    "last_frame_asset_id requires first_frame_asset_id to preserve frame order"
                        .to_owned(),
                );
            }
            Ok(())
        })
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_creation_audio(request: &CreationAudioRequest) -> Result<(), Wave3HostPortError> {
    validate_creation_target(&request.target, Some(CreationWorkbenchKind::Audio))
        .and_then(|_| require_bounded_text("text", &request.text, MAX_PROMPT_CHARS, false))
        .and_then(|_| validate_optional_short("voice", request.voice.as_deref(), 256))
        .and_then(|_| validate_optional_short("format", request.format.as_deref(), 64))
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_creation_target(
    target: &CreationTaskTarget,
    standalone_kind: Option<CreationWorkbenchKind>,
) -> Result<(), String> {
    match target {
        CreationTaskTarget::CanvasNode { canvas_id, node_id } => {
            require_uuidv7("target.canvas_id", canvas_id)?;
            require_uuidv7("target.node_id", node_id)
        }
        CreationTaskTarget::StandaloneWorkbench { workbench_kind } => {
            let expected = standalone_kind.ok_or_else(|| {
                "this creation action has no standalone workbench owner".to_owned()
            })?;
            if *workbench_kind != expected {
                return Err(format!(
                    "standalone workbench kind {} cannot own this action; expected {}",
                    workbench_kind.as_str(),
                    expected.as_str()
                ));
            }
            Ok(())
        }
        CreationTaskTarget::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        } => {
            require_uuidv7("target.template_id", template_id)?;
            require_uuidv7("target.template_run_id", template_run_id)?;
            require_uuidv7("target.template_step_id", template_step_id)
        }
    }
}

fn validate_creation_count(count: u32) -> Result<(), String> {
    if (1..=MAX_CREATION_RESULTS as u32).contains(&count) {
        Ok(())
    } else {
        Err(format!(
            "count must be between 1 and {MAX_CREATION_RESULTS}"
        ))
    }
}

fn require_bounded_text(
    label: &str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && value.is_empty()) || value.chars().count() > max_chars {
        return Err(format!(
            "{label} must contain {} to {max_chars} characters",
            if allow_empty { 0 } else { 1 }
        ));
    }
    Ok(())
}

fn validate_optional_short(
    label: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<(), String> {
    if let Some(value) = value {
        require_bounded_text(label, value, max_chars, false)?;
        if value.trim() != value {
            return Err(format!("{label} must be trimmed"));
        }
    }
    Ok(())
}

fn require_uuidv7(label: &str, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let canonical = bytes.len() == 36
        && [8, 13, 18, 23].into_iter().all(|index| bytes[index] == b'-')
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
        });
    if canonical {
        Ok(())
    } else {
        Err(format!("{label} must be a canonical lowercase UUIDv7"))
    }
}

fn validate_resource_bindings(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &[TypedResourceBinding],
) -> Result<(), KernelError> {
    validate_resource_bindings_contract(capability_id, principal_id, requirements, bindings)
        .map_err(|error| {
            if error.code == WAVE3_RESOURCE_OWNER_MISMATCH {
                let binding_id = bindings
                    .iter()
                    .find(|binding| binding.owner_id != principal_id)
                    .map(|binding| binding.binding_id.clone())
                    .unwrap_or_else(|| ResourceBindingId::from("unknown"));
                KernelError::ResourceOwnerMismatch { binding_id }
            } else if error.code == WAVE3_RESOURCE_NOT_BOUND {
                let resource_kind = requirements
                    .iter()
                    .find(|requirement| {
                        !bindings.iter().any(|binding| {
                            binding.resource_kind.as_ref() == requirement.resource_kind
                        })
                    })
                    .map(|requirement| requirement.resource_kind.to_owned())
                    .unwrap_or_else(|| "unknown".to_owned());
                KernelError::CapabilityResourceNotBound {
                    capability_id: capability_id.clone(),
                    resource_kind,
                }
            } else {
                wave3_host_error_to_kernel(error)
            }
        })
}

fn validate_host_context(context: &Wave3HostContext) -> Result<(), Wave3HostPortError> {
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
        return Err(Wave3HostPortError::invalid_request(format!(
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
) -> Result<(), Wave3HostPortError> {
    if principal_id.trim().is_empty() {
        return Err(Wave3HostPortError::invalid_request(
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
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
                "{} requires non-empty binding, resource kind, resource ID, and owner ID",
                capability_id.as_ref()
            )));
        }
        if !seen_binding_ids.insert(binding.binding_id.clone()) {
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource binding {}",
                capability_id.as_ref(),
                binding.binding_id.as_ref()
            )));
        }
        if binding.owner_id != principal_id {
            return Err(Wave3HostPortError::resource_owner_mismatch(format!(
                "resource binding {} belongs to {}, not {}",
                binding.binding_id.as_ref(),
                binding.owner_id,
                principal_id
            )));
        }
        if !seen_resource_kinds.insert(binding.resource_kind.clone()) {
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
                "{} received duplicate resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        if !expected_kinds.contains(&binding.resource_kind) {
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
                "{} received unexpected resource kind {}",
                capability_id.as_ref(),
                binding.resource_kind.as_ref()
            )));
        }
        let Some(allowed_operations) = declared_operations.get(&binding.resource_kind) else {
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
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
            return Err(Wave3HostPortError::new(
                WAVE3_RESOURCE_NOT_BOUND,
                format!(
                    "{} is missing resource kind {}",
                    capability_id.as_ref(),
                    requirement.resource_kind
                ),
            ));
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(Wave3HostPortError::resource_binding_invalid(format!(
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
            assert_eq!(
                manifest
                    .entrypoint
                    .as_in_process()
                    .expect("first-party entrypoint must be in-process")
                    .entrypoint_profile,
                "trusted-in-process"
            );
            assert_eq!(
                registration.handler_ids().len(),
                manifest.contributions.capabilities.len()
            );
            for capability in &manifest.contributions.capabilities {
                assert_eq!(capability.kind, CapabilityKind::Tool);
                assert_eq!(capability.contributions.actions.len(), 1);
                assert_eq!(
                    capability.contributions.actions[0].action_id.as_ref(),
                    format!("{}.invoke", capability.id.as_ref())
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
    fn creative_agent_actions_export_exact_resolvable_schemas_without_director() {
        let creative_ids = BTreeSet::from([
            "creation.audio",
            "creation.image",
            "creation.image_edit",
            "creation.text",
            "creation.video",
            "miniapp.edit",
            "miniapp.publish",
            "miniapp.read",
            "miniapp.serve",
            "workshop.asset.read",
            "workshop.asset.write",
            "workshop.canvas.edit",
            "workshop.canvas.read",
            "workshop.template.run",
        ]);
        assert!(!creative_ids.contains("workshop.director"));

        let manifests = registrations()
            .expect("registrations")
            .into_iter()
            .flat_map(|registration| registration.metadata.manifest.payload.contributions.capabilities)
            .map(|manifest| (manifest.id.as_ref().to_owned(), manifest))
            .collect::<BTreeMap<_, _>>();
        for capability_id in creative_ids {
            let manifest = manifests.get(capability_id).expect("creative manifest");
            let action = manifest
                .contributions
                .actions
                .first()
                .expect("creative action");
            for reference in [&action.input_schema, &action.output_schema] {
                let schema = resolve_action_schema(capability_id, reference)
                    .expect("manifest schema ref resolves from same source");
                assert_ne!(
                    schema.0.get("additionalProperties"),
                    Some(&json!(true)),
                    "{capability_id} must not publish the old permissive object schema"
                );
            }
        }
    }

    #[test]
    fn host_failures_preserve_their_canonical_code_in_kernel_results() {
        let error = wave3_host_error_to_kernel(Wave3HostPortError::new(
            "CREATION_OWNER_REJECTED",
            "the bound owner was rejected",
        ));
        let failure = error
            .capability_execution_failure()
            .expect("typed capability failure");
        assert_eq!(failure.code.as_ref(), "CREATION_OWNER_REJECTED");
        assert_eq!(failure.message, "the bound owner was rejected");
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
                operation_from_input(&capability_id, valid_input(capability.id)).is_ok(),
                "{} must have a host operation",
                capability.id
            );
        }
    }

    #[test]
    fn operation_mapping_and_resource_requirements_match_the_frozen_inventory() {
        for capability in PACKAGE_SPECS
            .iter()
            .flat_map(|package| package.capabilities.iter())
        {
            let capability_id = CapabilityId::from(capability.id);
            let expected_kinds = required_resource_kinds(capability.id).expect("known capability");
            let bindings = canonical_resource_bindings("wave3-test-owner")
                .into_iter()
                .filter(|binding| expected_kinds.contains(&binding.resource_kind))
                .collect::<Vec<_>>();
            let operation = operation_from_input(&capability_id, valid_input(capability.id))
                .expect("every Wave 3 capability has a typed operation");
            assert_eq!(operation.capability_id(), capability_id);
            assert_eq!(
                operation.action_id(),
                action_id(capability.id).expect("every Wave 3 capability has an action id")
            );

            match capability.id {
                "creation.text" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationText(_)
                    ));
                }
                "creation.image" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationImage(_)
                    ));
                }
                "creation.image_edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationImageEdit(_)
                    ));
                }
                "creation.video" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationVideo(_)
                    ));
                }
                "creation.audio" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::CreationAudio(_)
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
    fn composed_host_port_routes_by_owner_and_propagates_owner_errors() {
        struct RecordingOwner {
            domain: Wave3OwnerDomain,
            calls: Arc<Mutex<Vec<Wave3OwnerDomain>>>,
            error: Wave3HostPortError,
        }

        impl Wave3HostPort for RecordingOwner {
            fn invoke<'a>(
                &'a self,
                request: Wave3HostRequest,
            ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
            {
                let calls = Arc::clone(&self.calls);
                let domain = self.domain;
                let error = self.error.clone();
                Box::pin(async move {
                    request.validate()?;
                    calls.lock().expect("recording owner lock").push(domain);
                    Err(error)
                })
            }
        }

        fn poll_ready<F: Future>(future: F) -> F::Output {
            let waker = Waker::noop();
            let mut context = Context::from_waker(waker);
            let mut future = std::pin::pin!(future);
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("test owner must settle immediately"),
            }
        }

        fn request_for(capability_id: &str) -> Wave3HostRequest {
            let capability_id = CapabilityId::from(capability_id);
            let input = valid_input(capability_id.as_ref());
            let operation =
                operation_from_input(&capability_id, input).expect("known Wave 3 operation");
            let owner_id = "wave3-test-owner";
            let required_kinds = required_resource_kinds(capability_id.as_ref())
                .expect("known Wave 3 resource requirements");
            let resource_bindings = canonical_resource_bindings(owner_id)
                .into_iter()
                .filter(|binding| required_kinds.contains(&binding.resource_kind))
                .collect();
            Wave3HostRequest {
                context: Wave3HostContext {
                    principal: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: owner_id.to_owned(),
                    },
                    agent_session_id: AgentSessionId::from("wave3-test-session"),
                    operation_id: OperationId::from("wave3-test-operation"),
                    idempotency_key: IdempotencyKey::from("wave3-test-idempotency"),
                    correlation_id: CorrelationId::from("wave3-test-correlation"),
                    resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: "wave3-test-snapshot".into(),
                        snapshot_digest: "wave3-test-digest".into(),
                    },
                    registry_generation: 1,
                    capability_id: capability_id.clone(),
                    action_id: action_id(capability_id.as_ref()).expect("action id"),
                    state_scope_key: ScopeKey::from("session:wave3-test"),
                    resource_bindings,
                },
                operation,
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let host = composed_host_port(
            Wave3OwnerBindings::default().with_creation(Arc::new(RecordingOwner {
                domain: Wave3OwnerDomain::Creation,
                calls: Arc::clone(&calls),
                error: Wave3HostPortError::new(
                    "CREATION_OWNER_REACHED",
                    "Creation owner received the request",
                ),
            })),
        );
        let error = poll_ready(host.invoke(request_for("creation.text")))
            .expect_err("bound Creation owner should receive the request");
        assert_eq!(error.code, "CREATION_OWNER_REACHED");
        assert_eq!(
            *calls.lock().expect("recording owner lock"),
            vec![Wave3OwnerDomain::Creation]
        );

        let missing_owner = composed_host_port(Wave3OwnerBindings::default());
        let error = poll_ready(missing_owner.invoke(request_for("miniapp.read")))
            .expect_err("missing MiniApp owner must fail closed");
        assert_eq!(error.code, WAVE3_HOST_PORT_UNAVAILABLE);
        assert_eq!(
            error.message,
            "no production owner is bound for miniapp.read"
        );

        let owner_error = Wave3HostPortError::new("OWNER_ACTION_FAILED", "owner rejected action");
        let failing_host = composed_host_port(
            Wave3OwnerBindings::default().with_office(Arc::new(RecordingOwner {
                domain: Wave3OwnerDomain::Office,
                calls,
                error: owner_error.clone(),
            })),
        );
        assert_eq!(
            poll_ready(failing_host.invoke(request_for("office.preview")))
                .expect_err("owner errors must propagate unchanged"),
            owner_error
        );
    }

    #[test]
    fn host_request_validation_rejects_cross_capability_and_invalid_resource_operations() {
        let mut request = {
            let capability_id = CapabilityId::from("creation.text");
            let operation =
                operation_from_input(&capability_id, valid_input("creation.text")).unwrap();
            let mut request = {
                let owner_id = "wave3-test-owner";
                Wave3HostRequest {
                    context: Wave3HostContext {
                        principal: PrincipalRef {
                            principal_kind: "user".to_owned(),
                            principal_id: owner_id.to_owned(),
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
                        capability_id,
                        action_id: ActionId::from("creation.text.invoke"),
                        state_scope_key: ScopeKey::from("session:wave3-test"),
                        resource_bindings: canonical_resource_bindings(owner_id)
                            .into_iter()
                            .filter(|binding| {
                                binding.resource_kind.as_ref() == GENERATION_PROVIDER_RESOURCE_KIND
                            })
                            .collect(),
                    },
                    operation,
                }
            };
            request.context.action_id = ActionId::from("creation.image.invoke");
            request
        };
        let error = request
            .validate()
            .expect_err("cross-capability action identity must reject");
        assert_eq!(error.code, WAVE3_ACTION_OPERATION_MISMATCH);

        request.context.action_id = ActionId::from("creation.text.invoke");
        request.context.resource_bindings[0]
            .operations
            .insert("not-declared".to_owned());
        let error = request
            .validate()
            .expect_err("undeclared resource operation must reject");
        assert_eq!(error.code, WAVE3_RESOURCE_BINDING_INVALID);
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
                action_id: ActionId::from("creation.text.invoke"),
                state_scope_key: ScopeKey::from("session:wave3-test"),
                resource_bindings: canonical_resource_bindings("wave3-test-owner")
                    .into_iter()
                    .filter(|binding| {
                        binding.resource_kind.as_ref() == GENERATION_PROVIDER_RESOURCE_KIND
                    })
                    .collect(),
            },
            operation: Wave3CapabilityOperation::CreationText(CreationTextRequest {
                target: CreationTaskTarget::CanvasNode {
                    canvas_id: "0190f5fe-7c00-7a00-8000-000000000001".to_owned(),
                    node_id: "0190f5fe-7c00-7a00-8000-000000000002".to_owned(),
                },
                prompt: "hello".to_owned(),
                system: None,
                max_tokens: 4_096,
            }),
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

    fn valid_input(capability_id: &str) -> StrictJsonValue {
        const CANVAS_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
        const NODE_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";
        const ASSET_ID: &str = "0190f5fe-7c00-7a00-8000-000000000003";
        let target = json!({
            "kind": "canvas_node",
            "canvas_id": CANVAS_ID,
            "node_id": NODE_ID,
        });
        StrictJsonValue(match capability_id {
            "creation.text" => json!({"target": target, "prompt": "hello"}),
            "creation.image" => json!({"target": target, "prompt": "image"}),
            "creation.image_edit" => json!({
                "target": target,
                "prompt": "edit",
                "inputs": [{"asset_id": ASSET_ID, "role": "reference"}],
            }),
            "creation.video" => json!({"target": target, "prompt": "video"}),
            "creation.audio" => json!({"target": target, "text": "speak"}),
            _ => json!({}),
        })
    }
}
