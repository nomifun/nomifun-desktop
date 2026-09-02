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
use serde_json::{Map, Value, json};

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
pub const WAVE3_CONTRACT_NOT_FROZEN: &str = "WAVE3_CONTRACT_NOT_FROZEN";

const MAX_ACTION_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ACTION_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROMPT_CHARS: usize = 65_536;
const MAX_SYSTEM_CHARS: usize = 65_536;
const MAX_REVISION_CHARS: usize = 32;
const MAX_CREATION_RESULTS: usize = 10;
const MAX_IMAGE_EDIT_INPUTS: usize = 8;
const MAX_CANVAS_OPS: usize = 64;
const MAX_CANVAS_VALUE_BYTES: usize = 1024 * 1024;
const MAX_TEMPLATE_INPUTS: usize = 100;
const MAX_TEMPLATE_REFERENCES: usize = 100;
const MAX_TEMPLATE_TEXT_CHARS: usize = 20_000;
const MAX_TEMPLATE_RESULT_IDS: usize = 1_000;
const MAX_ASSET_TAGS: usize = 128;
const MAX_ASSET_TAG_CHARS: usize = 256;
const MAX_ASSET_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MINIAPP_NAME_CHARS: usize = 100;
const MAX_MINIAPP_DESCRIPTION_CHARS: usize = 500;
const MAX_MINIAPP_ICON_CHARS: usize = 16;
const MAX_MINIAPP_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_URL_CHARS: usize = 4_096;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationImageInput {
    pub asset_id: String,
    pub role: CreationImageInputRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationTextRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationImageRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub count: u32,
    pub size: Option<String>,
    pub quality: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationImageEditRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub inputs: Vec<CreationImageInput>,
    pub count: u32,
    pub size: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationVideoRequest {
    pub target: CreationTaskTarget,
    pub prompt: String,
    pub seconds: Option<u32>,
    pub size: Option<String>,
    pub first_frame_asset_id: Option<String>,
    pub last_frame_asset_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationAudioRequest {
    pub target: CreationTaskTarget,
    pub text: String,
    pub voice: Option<String>,
    pub format: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkshopCanvasReadRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopCanvasNodeType {
    Image,
    Panorama,
    Text,
    Config,
    Video,
    Audio,
    Director,
    Group,
}

impl WorkshopCanvasNodeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Panorama => "panorama",
            Self::Text => "text",
            Self::Config => "config",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Director => "director",
            Self::Group => "group",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkshopCanvasEditOperation {
    AddNode {
        node_type: WorkshopCanvasNodeType,
        x: f64,
        y: f64,
        width: Option<f64>,
        height: Option<f64>,
        group_id: Option<String>,
        data: StrictJsonValue,
    },
    UpdateNodeData {
        node_id: String,
        patch: StrictJsonValue,
    },
    MoveNode {
        node_id: String,
        x: f64,
        y: f64,
    },
    ResizeNode {
        node_id: String,
        width: f64,
        height: f64,
    },
    Connect {
        source_node_id: String,
        target_node_id: String,
        source_handle: Option<String>,
        target_handle: Option<String>,
    },
    Disconnect {
        connection_id: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkshopCanvasEditRequest {
    pub expected_revision: String,
    pub operations: Vec<WorkshopCanvasEditOperation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopAssetReadRequest {
    pub asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopAssetWriteRequest {
    CreateText {
        title: String,
        text_content: String,
        collection: Option<String>,
        tags: Vec<String>,
        in_library: bool,
    },
    UpdateMetadata {
        asset_id: String,
        title: Option<String>,
        collection: Option<String>,
        tags: Option<Vec<String>>,
        in_library: Option<bool>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkshopTemplateInputValue {
    Text {
        variable_id: String,
        value: String,
    },
    MultilineText {
        variable_id: String,
        value: String,
    },
    Number {
        variable_id: String,
        value: f64,
    },
    Boolean {
        variable_id: String,
        value: bool,
    },
    Choice {
        variable_id: String,
        value: String,
    },
    Image {
        variable_id: String,
        asset_id: Option<String>,
    },
    ImageSeries {
        variable_id: String,
        asset_ids: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkshopTemplateRunRequest {
    pub template_run_id: String,
    pub template_id: String,
    pub template_revision: i64,
    pub inputs: Vec<WorkshopTemplateInputValue>,
    pub reference_asset_ids: Vec<String>,
}

/// No canonical Director command exists in the production Workshop domain.
/// The uninhabited DTO keeps the action visible while making success
/// impossible until that owner contract is frozen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopDirectorRequest {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeDocumentType {
    Word,
    Excel,
    Ppt,
}

impl OfficeDocumentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::Excel => "excel",
            Self::Ppt => "ppt",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfficePreviewRequest {
    pub asset_id: String,
    pub document_type: OfficeDocumentType,
}

/// The current Office domain owns preview only. It has no document mutation
/// service that can define these three actions without guessing a wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeDocumentEditRequest {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeSheetEditRequest {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeSlidesEditRequest {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MiniAppReadRequest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppEditRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub html: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MiniAppPublishRequest;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MiniAppServeRequest;

/// Exact action-specific operations accepted by the Wave 3 host.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave3CapabilityOperation {
    CreationText(CreationTextRequest),
    CreationImage(CreationImageRequest),
    CreationImageEdit(CreationImageEditRequest),
    CreationVideo(CreationVideoRequest),
    CreationAudio(CreationAudioRequest),
    WorkshopCanvasRead(WorkshopCanvasReadRequest),
    WorkshopCanvasEdit(WorkshopCanvasEditRequest),
    WorkshopAssetRead(WorkshopAssetReadRequest),
    WorkshopAssetWrite(WorkshopAssetWriteRequest),
    WorkshopTemplateRun(WorkshopTemplateRunRequest),
    WorkshopDirector(WorkshopDirectorRequest),
    OfficePreview(OfficePreviewRequest),
    OfficeDocumentEdit(OfficeDocumentEditRequest),
    OfficeSheetEdit(OfficeSheetEditRequest),
    OfficeSlidesEdit(OfficeSlidesEditRequest),
    MiniAppRead(MiniAppReadRequest),
    MiniAppEdit(MiniAppEditRequest),
    MiniAppPublish(MiniAppPublishRequest),
    MiniAppServe(MiniAppServeRequest),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave3HostRequest {
    pub context: Wave3HostContext,
    pub operation: Wave3CapabilityOperation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreationTaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl CreationTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

macro_rules! creation_outcome {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name {
            pub task_id: String,
            pub status: CreationTaskStatus,
            pub result_asset_ids: Vec<String>,
        }
    };
}

creation_outcome!(CreationTextOutcome);
creation_outcome!(CreationImageOutcome);
creation_outcome!(CreationImageEditOutcome);
creation_outcome!(CreationVideoOutcome);
creation_outcome!(CreationAudioOutcome);

#[derive(Clone, Debug, PartialEq)]
pub struct WorkshopCanvasReadOutcome {
    pub canvas_id: String,
    pub revision: String,
    pub document_digest: String,
    pub document: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopCanvasEditOperationOutcome {
    NodeAdded { node_id: String },
    NodeUpdated { node_id: String },
    NodeMoved { node_id: String },
    NodeResized { node_id: String },
    NodesConnected { connection_id: String },
    NodesDisconnected { connection_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopCanvasEditOutcome {
    pub canvas_id: String,
    pub applied_revision: String,
    pub replayed: bool,
    pub operation_results: Vec<WorkshopCanvasEditOperationOutcome>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopAssetRecord {
    pub asset_id: String,
    pub kind: String,
    pub title: String,
    pub collection: Option<String>,
    pub tags: Vec<String>,
    pub mime: Option<String>,
    pub byte_size: Option<u64>,
    pub in_library: bool,
    pub text_content: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopAssetReadOutcome {
    pub asset: WorkshopAssetRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopAssetWriteOutcome {
    pub asset: WorkshopAssetRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopTemplateRunStatus {
    Requested,
    AwaitingReview,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl WorkshopTemplateRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::AwaitingReview => "awaiting-review",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopTemplateRunOutcome {
    pub template_run_id: String,
    pub template_id: String,
    pub revision: u64,
    pub status: WorkshopTemplateRunStatus,
    pub task_ids: Vec<String>,
    pub result_asset_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopDirectorOutcome {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfficePreviewOutcome {
    pub asset_id: String,
    pub document_type: OfficeDocumentType,
    pub preview_url: String,
    pub capability: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeDocumentEditOutcome {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeSheetEditOutcome {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeSlidesEditOutcome {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppRecord {
    pub miniapp_id: String,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub source_conversation_id: Option<String>,
    pub html_size: u64,
    pub published_at: Option<i64>,
    pub has_unpublished_changes: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppReadOutcome {
    pub app: MiniAppRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppEditOutcome {
    pub app: MiniAppRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppPublishOutcome {
    pub app: MiniAppRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppServeOutcome {
    pub miniapp_id: String,
    pub serve_url: String,
    pub published_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave3CapabilityOutcome {
    CreationText(CreationTextOutcome),
    CreationImage(CreationImageOutcome),
    CreationImageEdit(CreationImageEditOutcome),
    CreationVideo(CreationVideoOutcome),
    CreationAudio(CreationAudioOutcome),
    WorkshopCanvasRead(WorkshopCanvasReadOutcome),
    WorkshopCanvasEdit(WorkshopCanvasEditOutcome),
    WorkshopAssetRead(WorkshopAssetReadOutcome),
    WorkshopAssetWrite(WorkshopAssetWriteOutcome),
    WorkshopTemplateRun(WorkshopTemplateRunOutcome),
    WorkshopDirector(WorkshopDirectorOutcome),
    OfficePreview(OfficePreviewOutcome),
    OfficeDocumentEdit(OfficeDocumentEditOutcome),
    OfficeSheetEdit(OfficeSheetEditOutcome),
    OfficeSlidesEdit(OfficeSlidesEditOutcome),
    MiniAppRead(MiniAppReadOutcome),
    MiniAppEdit(MiniAppEditOutcome),
    MiniAppPublish(MiniAppPublishOutcome),
    MiniAppServe(MiniAppServeOutcome),
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
            Self::WorkshopCanvasRead(_) => "workshop.canvas.read",
            Self::WorkshopCanvasEdit(_) => "workshop.canvas.edit",
            Self::WorkshopAssetRead(_) => "workshop.asset.read",
            Self::WorkshopAssetWrite(_) => "workshop.asset.write",
            Self::WorkshopTemplateRun(_) => "workshop.template.run",
            Self::WorkshopDirector(_) => "workshop.director",
            Self::OfficePreview(_) => "office.preview",
            Self::OfficeDocumentEdit(_) => "office.document.edit",
            Self::OfficeSheetEdit(_) => "office.sheet.edit",
            Self::OfficeSlidesEdit(_) => "office.slides.edit",
            Self::MiniAppRead(_) => "miniapp.read",
            Self::MiniAppEdit(_) => "miniapp.edit",
            Self::MiniAppPublish(_) => "miniapp.publish",
            Self::MiniAppServe(_) => "miniapp.serve",
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
            Self::WorkshopCanvasRead(_)
            | Self::WorkshopCanvasEdit(_)
            | Self::WorkshopAssetRead(_)
            | Self::WorkshopAssetWrite(_)
            | Self::WorkshopTemplateRun(_)
            | Self::WorkshopDirector(_) => Wave3OwnerDomain::Workshop,
            Self::OfficePreview(_)
            | Self::OfficeDocumentEdit(_)
            | Self::OfficeSheetEdit(_)
            | Self::OfficeSlidesEdit(_) => Wave3OwnerDomain::Office,
            Self::MiniAppRead(_)
            | Self::MiniAppEdit(_)
            | Self::MiniAppPublish(_)
            | Self::MiniAppServe(_) => Wave3OwnerDomain::MiniApp,
        }
    }

    pub fn validate(&self) -> Result<(), Wave3HostPortError> {
        match self {
            Self::CreationText(request) => validate_creation_text(request),
            Self::CreationImage(request) => validate_creation_image(request),
            Self::CreationImageEdit(request) => validate_creation_image_edit(request),
            Self::CreationVideo(request) => validate_creation_video(request),
            Self::CreationAudio(request) => validate_creation_audio(request),
            Self::WorkshopCanvasRead(_) => Ok(()),
            Self::WorkshopCanvasEdit(request) => validate_canvas_edit(request),
            Self::WorkshopAssetRead(request) => validate_asset_read(request),
            Self::WorkshopAssetWrite(request) => validate_asset_write(request),
            Self::WorkshopTemplateRun(request) => validate_template_run(request),
            Self::WorkshopDirector(_) => Err(Wave3HostPortError::contract_not_frozen(
                "workshop.director has no canonical production command DTO",
            )),
            Self::OfficePreview(request) => validate_office_preview(request),
            Self::OfficeDocumentEdit(_) => Err(Wave3HostPortError::contract_not_frozen(
                "office.document.edit has no production mutation owner",
            )),
            Self::OfficeSheetEdit(_) => Err(Wave3HostPortError::contract_not_frozen(
                "office.sheet.edit has no production mutation owner",
            )),
            Self::OfficeSlidesEdit(_) => Err(Wave3HostPortError::contract_not_frozen(
                "office.slides.edit has no production mutation owner",
            )),
            Self::MiniAppRead(_) => Ok(()),
            Self::MiniAppEdit(request) => validate_miniapp_edit(request),
            Self::MiniAppPublish(_) | Self::MiniAppServe(_) => Ok(()),
        }
    }
}

impl Wave3CapabilityOutcome {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::CreationText(_) => "creation.text",
            Self::CreationImage(_) => "creation.image",
            Self::CreationImageEdit(_) => "creation.image_edit",
            Self::CreationVideo(_) => "creation.video",
            Self::CreationAudio(_) => "creation.audio",
            Self::WorkshopCanvasRead(_) => "workshop.canvas.read",
            Self::WorkshopCanvasEdit(_) => "workshop.canvas.edit",
            Self::WorkshopAssetRead(_) => "workshop.asset.read",
            Self::WorkshopAssetWrite(_) => "workshop.asset.write",
            Self::WorkshopTemplateRun(_) => "workshop.template.run",
            Self::WorkshopDirector(_) => "workshop.director",
            Self::OfficePreview(_) => "office.preview",
            Self::OfficeDocumentEdit(_) => "office.document.edit",
            Self::OfficeSheetEdit(_) => "office.sheet.edit",
            Self::OfficeSlidesEdit(_) => "office.slides.edit",
            Self::MiniAppRead(_) => "miniapp.read",
            Self::MiniAppEdit(_) => "miniapp.edit",
            Self::MiniAppPublish(_) => "miniapp.publish",
            Self::MiniAppServe(_) => "miniapp.serve",
        })
    }

    pub fn into_wire(self) -> StrictJsonValue {
        StrictJsonValue(match self {
            Self::CreationText(outcome) => creation_outcome_json(
                outcome.task_id,
                outcome.status,
                outcome.result_asset_ids,
            ),
            Self::CreationImage(outcome) => creation_outcome_json(
                outcome.task_id,
                outcome.status,
                outcome.result_asset_ids,
            ),
            Self::CreationImageEdit(outcome) => creation_outcome_json(
                outcome.task_id,
                outcome.status,
                outcome.result_asset_ids,
            ),
            Self::CreationVideo(outcome) => creation_outcome_json(
                outcome.task_id,
                outcome.status,
                outcome.result_asset_ids,
            ),
            Self::CreationAudio(outcome) => creation_outcome_json(
                outcome.task_id,
                outcome.status,
                outcome.result_asset_ids,
            ),
            Self::WorkshopCanvasRead(outcome) => json!({
                "canvas_id": outcome.canvas_id,
                "revision": outcome.revision,
                "document_digest": outcome.document_digest,
                "document": outcome.document.0,
            }),
            Self::WorkshopCanvasEdit(outcome) => json!({
                "canvas_id": outcome.canvas_id,
                "applied_revision": outcome.applied_revision,
                "replayed": outcome.replayed,
                "operation_results": outcome
                    .operation_results
                    .into_iter()
                    .map(canvas_edit_outcome_json)
                    .collect::<Vec<_>>(),
            }),
            Self::WorkshopAssetRead(outcome) => {
                json!({"asset": workshop_asset_json(outcome.asset)})
            }
            Self::WorkshopAssetWrite(outcome) => {
                json!({"asset": workshop_asset_json(outcome.asset)})
            }
            Self::WorkshopTemplateRun(outcome) => json!({
                "template_run_id": outcome.template_run_id,
                "template_id": outcome.template_id,
                "revision": outcome.revision,
                "status": outcome.status.as_str(),
                "task_ids": outcome.task_ids,
                "result_asset_ids": outcome.result_asset_ids,
            }),
            Self::WorkshopDirector(outcome) => match outcome {},
            Self::OfficePreview(outcome) => json!({
                "asset_id": outcome.asset_id,
                "document_type": outcome.document_type.as_str(),
                "preview_url": outcome.preview_url,
                "capability": outcome.capability,
            }),
            Self::OfficeDocumentEdit(outcome) => match outcome {},
            Self::OfficeSheetEdit(outcome) => match outcome {},
            Self::OfficeSlidesEdit(outcome) => match outcome {},
            Self::MiniAppRead(outcome) => json!({"app": miniapp_record_json(outcome.app)}),
            Self::MiniAppEdit(outcome) => json!({"app": miniapp_record_json(outcome.app)}),
            Self::MiniAppPublish(outcome) => json!({"app": miniapp_record_json(outcome.app)}),
            Self::MiniAppServe(outcome) => json!({
                "miniapp_id": outcome.miniapp_id,
                "serve_url": outcome.serve_url,
                "published_at": outcome.published_at,
            }),
        })
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

    pub fn contract_not_frozen(message: impl Into<String>) -> Self {
        Self::new(WAVE3_CONTRACT_NOT_FROZEN, message)
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
    let input_schema = action_input_schema(spec.id);
    let output_schema = action_output_schema(spec.id);
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
        supported_surfaces: AGENT_SURFACES
            .iter()
            .map(|surface| (*surface).to_owned())
            .collect(),
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

fn strict_object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn string_schema(max_length: usize) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max_length})
}

fn nullable_string_schema(max_length: usize) -> Value {
    json!({
        "oneOf": [
            {"type": "string", "maxLength": max_length},
            {"type": "null"}
        ]
    })
}

fn uuidv7_schema() -> Value {
    json!({
        "type": "string",
        "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
    })
}

fn empty_action_schema() -> Value {
    strict_object_schema(json!({}), &[])
}

fn blocked_action_schema(reason: &str) -> Value {
    json!({
        "not": {},
        "description": reason,
    })
}

fn creation_target_schema() -> Value {
    json!({
        "oneOf": [
            strict_object_schema(
                json!({
                    "kind": {"const": "canvas_node"},
                    "canvas_id": uuidv7_schema(),
                    "node_id": uuidv7_schema()
                }),
                &["kind", "canvas_id", "node_id"]
            ),
            strict_object_schema(
                json!({
                    "kind": {"const": "standalone_workbench"},
                    "workbench_kind": {"enum": ["image", "video", "audio"]}
                }),
                &["kind", "workbench_kind"]
            ),
            strict_object_schema(
                json!({
                    "kind": {"const": "template_step"},
                    "template_id": uuidv7_schema(),
                    "template_run_id": uuidv7_schema(),
                    "template_step_id": uuidv7_schema()
                }),
                &["kind", "template_id", "template_run_id", "template_step_id"]
            )
        ]
    })
}

fn canvas_edit_operation_schema() -> Value {
    let number = json!({"type": "number"});
    json!({
        "oneOf": [
            strict_object_schema(
                json!({
                    "type": {"const": "add_node"},
                    "node_type": {
                        "enum": ["image", "panorama", "text", "config", "video", "audio", "director", "group"]
                    },
                    "x": number,
                    "y": {"type": "number"},
                    "width": {"type": "number", "minimum": 1},
                    "height": {"type": "number", "minimum": 1},
                    "group_id": uuidv7_schema(),
                    "data": {"type": "object", "maxProperties": 4096}
                }),
                &["type", "node_type", "x", "y", "data"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "update_node_data"},
                    "node_id": uuidv7_schema(),
                    "patch": {"type": "object", "maxProperties": 4096}
                }),
                &["type", "node_id", "patch"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "move_node"},
                    "node_id": uuidv7_schema(),
                    "x": {"type": "number"},
                    "y": {"type": "number"}
                }),
                &["type", "node_id", "x", "y"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "resize_node"},
                    "node_id": uuidv7_schema(),
                    "width": {"type": "number", "minimum": 1},
                    "height": {"type": "number", "minimum": 1}
                }),
                &["type", "node_id", "width", "height"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "connect"},
                    "source_node_id": uuidv7_schema(),
                    "target_node_id": uuidv7_schema(),
                    "source_handle": string_schema(256),
                    "target_handle": string_schema(256)
                }),
                &["type", "source_node_id", "target_node_id"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "disconnect"},
                    "connection_id": uuidv7_schema()
                }),
                &["type", "connection_id"]
            )
        ]
    })
}

fn template_input_schema() -> Value {
    json!({
        "oneOf": [
            strict_object_schema(
                json!({
                    "type": {"enum": ["text", "multiline-text", "choice"]},
                    "variable_id": uuidv7_schema(),
                    "value": string_schema(MAX_TEMPLATE_TEXT_CHARS)
                }),
                &["type", "variable_id", "value"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "number"},
                    "variable_id": uuidv7_schema(),
                    "value": {"type": "number"}
                }),
                &["type", "variable_id", "value"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "boolean"},
                    "variable_id": uuidv7_schema(),
                    "value": {"type": "boolean"}
                }),
                &["type", "variable_id", "value"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "image"},
                    "variable_id": uuidv7_schema(),
                    "asset_id": uuidv7_schema()
                }),
                &["type", "variable_id"]
            ),
            strict_object_schema(
                json!({
                    "type": {"const": "image-series"},
                    "variable_id": uuidv7_schema(),
                    "asset_ids": {
                        "type": "array",
                        "maxItems": MAX_TEMPLATE_REFERENCES,
                        "uniqueItems": true,
                        "items": uuidv7_schema()
                    }
                }),
                &["type", "variable_id", "asset_ids"]
            )
        ]
    })
}

fn action_input_schema(capability_id: &str) -> Value {
    match capability_id {
        "creation.text" => strict_object_schema(
            json!({
                "target": creation_target_schema(),
                "prompt": string_schema(MAX_PROMPT_CHARS),
                "system": {"type": "string", "maxLength": MAX_SYSTEM_CHARS},
                "max_tokens": {"type": "integer", "minimum": 1, "maximum": 131072}
            }),
            &["target", "prompt"],
        ),
        "creation.image" => strict_object_schema(
            json!({
                "target": creation_target_schema(),
                "prompt": string_schema(MAX_PROMPT_CHARS),
                "count": {"type": "integer", "minimum": 1, "maximum": MAX_CREATION_RESULTS},
                "size": string_schema(128),
                "quality": string_schema(128)
            }),
            &["target", "prompt"],
        ),
        "creation.image_edit" => strict_object_schema(
            json!({
                "target": creation_target_schema(),
                "prompt": string_schema(MAX_PROMPT_CHARS),
                "inputs": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_IMAGE_EDIT_INPUTS,
                    "items": strict_object_schema(
                        json!({
                            "asset_id": uuidv7_schema(),
                            "role": {"enum": ["reference", "mask"]}
                        }),
                        &["asset_id", "role"]
                    )
                },
                "count": {"type": "integer", "minimum": 1, "maximum": MAX_CREATION_RESULTS},
                "size": string_schema(128)
            }),
            &["target", "prompt", "inputs"],
        ),
        "creation.video" => strict_object_schema(
            json!({
                "target": creation_target_schema(),
                "prompt": string_schema(MAX_PROMPT_CHARS),
                "seconds": {"type": "integer", "minimum": 1, "maximum": 3600},
                "size": string_schema(128),
                "first_frame_asset_id": uuidv7_schema(),
                "last_frame_asset_id": uuidv7_schema()
            }),
            &["target", "prompt"],
        ),
        "creation.audio" => strict_object_schema(
            json!({
                "target": creation_target_schema(),
                "text": string_schema(MAX_PROMPT_CHARS),
                "voice": string_schema(256),
                "format": string_schema(64)
            }),
            &["target", "text"],
        ),
        "workshop.canvas.read" | "miniapp.read" | "miniapp.publish" | "miniapp.serve" => {
            empty_action_schema()
        }
        "workshop.canvas.edit" => strict_object_schema(
            json!({
                "expected_revision": {
                    "type": "string",
                    "pattern": "^[1-9][0-9]{0,31}$"
                },
                "operations": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_CANVAS_OPS,
                    "items": canvas_edit_operation_schema()
                }
            }),
            &["expected_revision", "operations"],
        ),
        "workshop.asset.read" => {
            strict_object_schema(json!({"asset_id": uuidv7_schema()}), &["asset_id"])
        }
        "workshop.asset.write" => json!({
            "oneOf": [
                strict_object_schema(
                    json!({
                        "operation": {"const": "create_text"},
                        "title": string_schema(1000),
                        "text_content": {"type": "string", "maxLength": MAX_ASSET_TEXT_BYTES},
                        "collection": {"type": "string", "maxLength": 1000},
                        "tags": {
                            "type": "array",
                            "maxItems": MAX_ASSET_TAGS,
                            "uniqueItems": true,
                            "items": string_schema(MAX_ASSET_TAG_CHARS)
                        },
                        "in_library": {"type": "boolean"}
                    }),
                    &["operation", "title", "text_content"]
                ),
                strict_object_schema(
                    json!({
                        "operation": {"const": "update_metadata"},
                        "asset_id": uuidv7_schema(),
                        "title": string_schema(1000),
                        "collection": {"type": "string", "maxLength": 1000},
                        "tags": {
                            "type": "array",
                            "maxItems": MAX_ASSET_TAGS,
                            "uniqueItems": true,
                            "items": string_schema(MAX_ASSET_TAG_CHARS)
                        },
                        "in_library": {"type": "boolean"}
                    }),
                    &["operation", "asset_id"]
                )
            ]
        }),
        "workshop.template.run" => strict_object_schema(
            json!({
                "template_run_id": uuidv7_schema(),
                "template_id": uuidv7_schema(),
                "template_revision": {"type": "integer", "minimum": 1},
                "inputs": {
                    "type": "array",
                    "maxItems": MAX_TEMPLATE_INPUTS,
                    "items": template_input_schema()
                },
                "reference_asset_ids": {
                    "type": "array",
                    "maxItems": MAX_TEMPLATE_REFERENCES,
                    "uniqueItems": true,
                    "items": uuidv7_schema()
                }
            }),
            &[
                "template_run_id",
                "template_id",
                "template_revision",
                "inputs",
                "reference_asset_ids",
            ],
        ),
        "workshop.director" => blocked_action_schema(
            "No canonical production Director command DTO is frozen.",
        ),
        "office.preview" => strict_object_schema(
            json!({
                "asset_id": uuidv7_schema(),
                "document_type": {"enum": ["word", "excel", "ppt"]}
            }),
            &["asset_id", "document_type"],
        ),
        "office.document.edit" | "office.sheet.edit" | "office.slides.edit" => {
            blocked_action_schema("The production Office domain has no mutation owner.")
        }
        "miniapp.edit" => {
            let mut schema = strict_object_schema(
                json!({
                    "name": string_schema(MAX_MINIAPP_NAME_CHARS),
                    "description": {"type": "string", "maxLength": MAX_MINIAPP_DESCRIPTION_CHARS},
                    "icon": {"type": "string", "maxLength": MAX_MINIAPP_ICON_CHARS},
                    "html": string_schema(MAX_MINIAPP_HTML_BYTES)
                }),
                &[],
            );
            schema["minProperties"] = json!(1);
            schema
        }
        _ => blocked_action_schema("Unknown Wave 3 action."),
    }
}

fn creation_output_schema() -> Value {
    strict_object_schema(
        json!({
            "creation_task_id": uuidv7_schema(),
            "status": {"enum": ["queued", "running", "succeeded", "failed", "canceled"]},
            "result_asset_ids": {
                "type": "array",
                "maxItems": MAX_CREATION_RESULTS,
                "uniqueItems": true,
                "items": uuidv7_schema()
            }
        }),
        &["creation_task_id", "status", "result_asset_ids"],
    )
}

fn asset_record_schema() -> Value {
    strict_object_schema(
        json!({
            "asset_id": uuidv7_schema(),
            "kind": string_schema(32),
            "title": string_schema(1000),
            "collection": nullable_string_schema(1000),
            "tags": {
                "type": "array",
                "maxItems": MAX_ASSET_TAGS,
                "uniqueItems": true,
                "items": string_schema(MAX_ASSET_TAG_CHARS)
            },
            "mime": nullable_string_schema(256),
            "byte_size": {
                "oneOf": [
                    {"type": "integer", "minimum": 0},
                    {"type": "null"}
                ]
            },
            "in_library": {"type": "boolean"},
            "text_content": nullable_string_schema(MAX_ASSET_TEXT_BYTES)
        }),
        &[
            "asset_id",
            "kind",
            "title",
            "collection",
            "tags",
            "mime",
            "byte_size",
            "in_library",
            "text_content",
        ],
    )
}

fn miniapp_record_schema() -> Value {
    strict_object_schema(
        json!({
            "miniapp_id": uuidv7_schema(),
            "name": string_schema(MAX_MINIAPP_NAME_CHARS),
            "description": {"type": "string", "maxLength": MAX_MINIAPP_DESCRIPTION_CHARS},
            "icon": nullable_string_schema(MAX_MINIAPP_ICON_CHARS),
            "source_conversation_id": {
                "oneOf": [uuidv7_schema(), {"type": "null"}]
            },
            "html_size": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_MINIAPP_HTML_BYTES
            },
            "published_at": {
                "oneOf": [
                    {"type": "integer", "minimum": 0},
                    {"type": "null"}
                ]
            },
            "has_unpublished_changes": {"type": "boolean"},
            "created_at": {"type": "integer", "minimum": 0},
            "updated_at": {"type": "integer", "minimum": 0}
        }),
        &[
            "miniapp_id",
            "name",
            "description",
            "icon",
            "source_conversation_id",
            "html_size",
            "published_at",
            "has_unpublished_changes",
            "created_at",
            "updated_at",
        ],
    )
}

fn action_output_schema(capability_id: &str) -> Value {
    match capability_id {
        "creation.text"
        | "creation.image"
        | "creation.image_edit"
        | "creation.video"
        | "creation.audio" => creation_output_schema(),
        "workshop.canvas.read" => strict_object_schema(
            json!({
                "canvas_id": uuidv7_schema(),
                "revision": {"type": "string", "pattern": "^[1-9][0-9]{0,31}$"},
                "document_digest": {
                    "type": "string",
                    "pattern": "^[0-9a-f]{64}$"
                },
                "document": {"type": "object"}
            }),
            &["canvas_id", "revision", "document_digest", "document"],
        ),
        "workshop.canvas.edit" => strict_object_schema(
            json!({
                "canvas_id": uuidv7_schema(),
                "applied_revision": {"type": "string", "pattern": "^[1-9][0-9]{0,31}$"},
                "replayed": {"type": "boolean"},
                "operation_results": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_CANVAS_OPS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "type": {
                                "enum": [
                                    "node_added",
                                    "node_updated",
                                    "node_moved",
                                    "node_resized",
                                    "nodes_connected",
                                    "nodes_disconnected"
                                ]
                            },
                            "node_id": uuidv7_schema(),
                            "connection_id": uuidv7_schema()
                        },
                        "required": ["type"]
                    }
                }
            }),
            &[
                "canvas_id",
                "applied_revision",
                "replayed",
                "operation_results",
            ],
        ),
        "workshop.asset.read" | "workshop.asset.write" => {
            strict_object_schema(json!({"asset": asset_record_schema()}), &["asset"])
        }
        "workshop.template.run" => strict_object_schema(
            json!({
                "template_run_id": uuidv7_schema(),
                "template_id": uuidv7_schema(),
                "revision": {"type": "integer", "minimum": 1},
                "status": {
                    "enum": [
                        "requested",
                        "awaiting-review",
                        "queued",
                        "running",
                        "succeeded",
                        "failed",
                        "cancelled"
                    ]
                },
                "task_ids": {
                    "type": "array",
                    "maxItems": MAX_TEMPLATE_RESULT_IDS,
                    "uniqueItems": true,
                    "items": uuidv7_schema()
                },
                "result_asset_ids": {
                    "type": "array",
                    "maxItems": MAX_TEMPLATE_RESULT_IDS,
                    "uniqueItems": true,
                    "items": uuidv7_schema()
                }
            }),
            &[
                "template_run_id",
                "template_id",
                "revision",
                "status",
                "task_ids",
                "result_asset_ids",
            ],
        ),
        "workshop.director" => blocked_action_schema(
            "No canonical production Director outcome DTO is frozen.",
        ),
        "office.preview" => strict_object_schema(
            json!({
                "asset_id": uuidv7_schema(),
                "document_type": {"enum": ["word", "excel", "ppt"]},
                "preview_url": string_schema(MAX_URL_CHARS),
                "capability": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
            }),
            &["asset_id", "document_type", "preview_url", "capability"],
        ),
        "office.document.edit" | "office.sheet.edit" | "office.slides.edit" => {
            blocked_action_schema("The production Office domain has no mutation outcome.")
        }
        "miniapp.read" | "miniapp.edit" | "miniapp.publish" => {
            strict_object_schema(json!({"app": miniapp_record_schema()}), &["app"])
        }
        "miniapp.serve" => strict_object_schema(
            json!({
                "miniapp_id": uuidv7_schema(),
                "serve_url": string_schema(MAX_URL_CHARS),
                "published_at": {"type": "integer", "minimum": 0}
            }),
            &["miniapp_id", "serve_url", "published_at"],
        ),
        _ => blocked_action_schema("Unknown Wave 3 action outcome."),
    }
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
            .map(|capability_id| action_input_schema(capability_id))
            .collect::<Vec<_>>()
    });
    let response_schema = json!({
        "anyOf": ALL_CAPABILITY_IDS
            .iter()
            .map(|capability_id| action_output_schema(capability_id))
            .collect::<Vec<_>>()
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
            let outcome_bindings = request.context.resource_bindings.clone();
            request
                .validate()
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?;
            let result = self
                .host_port
                .invoke(request)
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?;
            let outcome = outcome_from_output(&self.capability_id, result).map_err(|error| {
                KernelError::CapabilityExecution {
                    reason: error.to_string(),
                }
            })?;
            validate_outcome_resource_binding(&outcome, &outcome_bindings).map_err(|error| {
                KernelError::CapabilityExecution {
                    reason: error.to_string(),
                }
            })?;
            Ok(outcome.into_wire())
        })
    }
}

/// Convert a canonical capability ID and its object payload into the only
/// typed operation variant accepted by the host port.
pub fn operation_from_input(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave3CapabilityOperation, KernelError> {
    validate_json_size(
        capability_id.as_ref(),
        "input",
        &input.0,
        MAX_ACTION_INPUT_BYTES,
    )
    .map_err(kernel_invalid_payload)?;
    let operation = match capability_id.as_ref() {
        "creation.text" => {
            Wave3CapabilityOperation::CreationText(parse_creation_text(input.0).map_err(
                kernel_invalid_payload,
            )?)
        }
        "creation.image" => {
            Wave3CapabilityOperation::CreationImage(parse_creation_image(input.0).map_err(
                kernel_invalid_payload,
            )?)
        }
        "creation.image_edit" => Wave3CapabilityOperation::CreationImageEdit(
            parse_creation_image_edit(input.0).map_err(kernel_invalid_payload)?,
        ),
        "creation.video" => {
            Wave3CapabilityOperation::CreationVideo(parse_creation_video(input.0).map_err(
                kernel_invalid_payload,
            )?)
        }
        "creation.audio" => {
            Wave3CapabilityOperation::CreationAudio(parse_creation_audio(input.0).map_err(
                kernel_invalid_payload,
            )?)
        }
        "workshop.canvas.read" => {
            parse_empty_request("workshop.canvas.read", input.0).map_err(kernel_invalid_payload)?;
            Wave3CapabilityOperation::WorkshopCanvasRead(WorkshopCanvasReadRequest)
        }
        "workshop.canvas.edit" => Wave3CapabilityOperation::WorkshopCanvasEdit(
            parse_canvas_edit(input.0).map_err(kernel_invalid_payload)?,
        ),
        "workshop.asset.read" => Wave3CapabilityOperation::WorkshopAssetRead(
            parse_asset_read(input.0).map_err(kernel_invalid_payload)?,
        ),
        "workshop.asset.write" => Wave3CapabilityOperation::WorkshopAssetWrite(
            parse_asset_write(input.0).map_err(kernel_invalid_payload)?,
        ),
        "workshop.template.run" => Wave3CapabilityOperation::WorkshopTemplateRun(
            parse_template_run(input.0).map_err(kernel_invalid_payload)?,
        ),
        "workshop.director" => {
            return Err(kernel_contract_not_frozen(
                "workshop.director has no canonical production command DTO; the current product \
                 persists Director state through Canvas documents and text-asset sidecars",
            ));
        }
        "office.preview" => Wave3CapabilityOperation::OfficePreview(
            parse_office_preview(input.0).map_err(kernel_invalid_payload)?,
        ),
        "office.document.edit" | "office.sheet.edit" | "office.slides.edit" => {
            return Err(kernel_contract_not_frozen(format!(
                "{} has no production Office mutation owner; the current Office domain exposes \
                 preview and snapshot services only",
                capability_id.as_ref()
            )));
        }
        "miniapp.read" => {
            parse_empty_request("miniapp.read", input.0).map_err(kernel_invalid_payload)?;
            Wave3CapabilityOperation::MiniAppRead(MiniAppReadRequest)
        }
        "miniapp.edit" => Wave3CapabilityOperation::MiniAppEdit(
            parse_miniapp_edit(input.0).map_err(kernel_invalid_payload)?,
        ),
        "miniapp.publish" => {
            parse_empty_request("miniapp.publish", input.0).map_err(kernel_invalid_payload)?;
            Wave3CapabilityOperation::MiniAppPublish(MiniAppPublishRequest)
        }
        "miniapp.serve" => {
            parse_empty_request("miniapp.serve", input.0).map_err(kernel_invalid_payload)?;
            Wave3CapabilityOperation::MiniAppServe(MiniAppServeRequest)
        }
        other => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{other} does not expose an action host operation"),
            });
        }
    };
    operation
        .validate()
        .map_err(|error| kernel_invalid_payload(error.message))?;
    Ok(operation)
}

/// Decode and canonicalize the action-specific result returned by a real
/// production owner. Unknown fields and cross-action result shapes reject.
pub fn outcome_from_output(
    capability_id: &CapabilityId,
    output: StrictJsonValue,
) -> Result<Wave3CapabilityOutcome, Wave3HostPortError> {
    validate_json_size(
        capability_id.as_ref(),
        "output",
        &output.0,
        MAX_ACTION_OUTPUT_BYTES,
    )
    .map_err(Wave3HostPortError::invalid_response)?;
    let outcome = match capability_id.as_ref() {
        "creation.text" => Wave3CapabilityOutcome::CreationText(
            parse_creation_outcome(output.0)
                .map(|(task_id, status, result_asset_ids)| CreationTextOutcome {
                    task_id,
                    status,
                    result_asset_ids,
                })
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "creation.image" => Wave3CapabilityOutcome::CreationImage(
            parse_creation_outcome(output.0)
                .map(|(task_id, status, result_asset_ids)| CreationImageOutcome {
                    task_id,
                    status,
                    result_asset_ids,
                })
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "creation.image_edit" => Wave3CapabilityOutcome::CreationImageEdit(
            parse_creation_outcome(output.0)
                .map(
                    |(task_id, status, result_asset_ids)| CreationImageEditOutcome {
                        task_id,
                        status,
                        result_asset_ids,
                    },
                )
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "creation.video" => Wave3CapabilityOutcome::CreationVideo(
            parse_creation_outcome(output.0)
                .map(|(task_id, status, result_asset_ids)| CreationVideoOutcome {
                    task_id,
                    status,
                    result_asset_ids,
                })
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "creation.audio" => Wave3CapabilityOutcome::CreationAudio(
            parse_creation_outcome(output.0)
                .map(|(task_id, status, result_asset_ids)| CreationAudioOutcome {
                    task_id,
                    status,
                    result_asset_ids,
                })
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "workshop.canvas.read" => Wave3CapabilityOutcome::WorkshopCanvasRead(
            parse_canvas_read_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "workshop.canvas.edit" => Wave3CapabilityOutcome::WorkshopCanvasEdit(
            parse_canvas_edit_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "workshop.asset.read" => {
            Wave3CapabilityOutcome::WorkshopAssetRead(WorkshopAssetReadOutcome {
                asset: parse_asset_outcome(output.0)
                    .map_err(Wave3HostPortError::invalid_response)?,
            })
        }
        "workshop.asset.write" => {
            Wave3CapabilityOutcome::WorkshopAssetWrite(WorkshopAssetWriteOutcome {
                asset: parse_asset_outcome(output.0)
                    .map_err(Wave3HostPortError::invalid_response)?,
            })
        }
        "workshop.template.run" => Wave3CapabilityOutcome::WorkshopTemplateRun(
            parse_template_run_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "workshop.director" => {
            return Err(Wave3HostPortError::contract_not_frozen(
                "workshop.director cannot return a successful outcome until its production \
                 command DTO is frozen",
            ));
        }
        "office.preview" => Wave3CapabilityOutcome::OfficePreview(
            parse_office_preview_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        "office.document.edit" | "office.sheet.edit" | "office.slides.edit" => {
            return Err(Wave3HostPortError::contract_not_frozen(format!(
                "{} cannot return a successful outcome without a production Office mutation owner",
                capability_id.as_ref()
            )));
        }
        "miniapp.read" => Wave3CapabilityOutcome::MiniAppRead(MiniAppReadOutcome {
            app: parse_miniapp_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        }),
        "miniapp.edit" => Wave3CapabilityOutcome::MiniAppEdit(MiniAppEditOutcome {
            app: parse_miniapp_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        }),
        "miniapp.publish" => {
            Wave3CapabilityOutcome::MiniAppPublish(MiniAppPublishOutcome {
                app: parse_miniapp_outcome(output.0)
                    .map_err(Wave3HostPortError::invalid_response)?,
            })
        }
        "miniapp.serve" => Wave3CapabilityOutcome::MiniAppServe(
            parse_miniapp_serve_outcome(output.0)
                .map_err(Wave3HostPortError::invalid_response)?,
        ),
        other => {
            return Err(Wave3HostPortError::invalid_response(format!(
                "{other} does not expose a Wave 3 outcome contract"
            )));
        }
    };
    Ok(outcome)
}

fn kernel_invalid_payload(message: impl Into<String>) -> KernelError {
    KernelError::CapabilityExecution {
        reason: format!("{WAVE3_INVALID_REQUEST}: {}", message.into()),
    }
}

fn kernel_contract_not_frozen(message: impl Into<String>) -> KernelError {
    KernelError::CapabilityExecution {
        reason: format!("{WAVE3_CONTRACT_NOT_FROZEN}: {}", message.into()),
    }
}

fn validate_json_size(
    capability_id: &str,
    facet: &str,
    value: &Value,
    max_bytes: usize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("{capability_id} {facet} is not serializable JSON: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "{capability_id} {facet} exceeds the {max_bytes}-byte limit"
        ));
    }
    Ok(())
}

fn object(value: Value, label: &str) -> Result<Map<String, Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} must be a JSON object"))
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("{label} contains unknown field `{field}`"));
    }
    Ok(())
}

fn required_value(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Value, String> {
    object
        .remove(field)
        .ok_or_else(|| format!("{label} requires field `{field}`"))
}

fn required_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = required_value(object, field, label)?;
    parse_string(value, &format!("{label}.{field}"), max_chars, false, true)
}

fn required_untrimmed_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = required_value(object, field, label)?;
    parse_string(value, &format!("{label}.{field}"), max_chars, false, false)
}

fn optional_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    object
        .remove(field)
        .map(|value| parse_string(value, &format!("{label}.{field}"), max_chars, false, true))
        .transpose()
}

fn optional_clearable_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    object
        .remove(field)
        .map(|value| parse_string(value, &format!("{label}.{field}"), max_chars, true, true))
        .transpose()
}

fn optional_nullable_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<Option<String>, String> {
    match object.remove(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => parse_string(
            value,
            &format!("{label}.{field}"),
            max_chars,
            allow_empty,
            true,
        )
        .map(Some),
    }
}

fn parse_string(
    value: Value,
    label: &str,
    max_chars: usize,
    allow_empty: bool,
    require_trimmed: bool,
) -> Result<String, String> {
    let value = value
        .as_str()
        .ok_or_else(|| format!("{label} must be a string"))?
        .to_owned();
    if (!allow_empty && value.is_empty()) || value.chars().count() > max_chars {
        return Err(format!(
            "{label} must contain {} to {max_chars} characters",
            if allow_empty { 0 } else { 1 }
        ));
    }
    if require_trimmed && value.trim() != value {
        return Err(format!("{label} must be trimmed"));
    }
    Ok(value)
}

fn required_bool(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<bool, String> {
    required_value(object, field, label)?
        .as_bool()
        .ok_or_else(|| format!("{label}.{field} must be a boolean"))
}

fn optional_bool(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    default: bool,
) -> Result<bool, String> {
    match object.remove(field) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("{label}.{field} must be a boolean")),
    }
}

fn optional_nullable_bool(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<bool>, String> {
    match object.remove(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("{label}.{field} must be a boolean or null")),
    }
}

fn required_u64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<u64, String> {
    required_value(object, field, label)?
        .as_u64()
        .ok_or_else(|| format!("{label}.{field} must be a non-negative integer"))
}

fn optional_u64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<u64>, String> {
    object
        .remove(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| format!("{label}.{field} must be a non-negative integer"))
        })
        .transpose()
}

fn required_i64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<i64, String> {
    required_value(object, field, label)?
        .as_i64()
        .ok_or_else(|| format!("{label}.{field} must be an integer"))
}

fn optional_nullable_i64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<i64>, String> {
    match object.remove(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{label}.{field} must be an integer or null")),
    }
}

fn required_f64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<f64, String> {
    let value = required_value(object, field, label)?
        .as_f64()
        .ok_or_else(|| format!("{label}.{field} must be a number"))?;
    if !value.is_finite() {
        return Err(format!("{label}.{field} must be finite"));
    }
    Ok(value)
}

fn optional_f64(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, String> {
    let Some(value) = object.remove(field) else {
        return Ok(None);
    };
    let value = value
        .as_f64()
        .ok_or_else(|| format!("{label}.{field} must be a number"))?;
    if !value.is_finite() {
        return Err(format!("{label}.{field} must be finite"));
    }
    Ok(Some(value))
}

fn required_array(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_items: usize,
) -> Result<Vec<Value>, String> {
    let values = required_value(object, field, label)?
        .as_array()
        .cloned()
        .ok_or_else(|| format!("{label}.{field} must be an array"))?;
    if values.len() > max_items {
        return Err(format!(
            "{label}.{field} contains {} entries (max {max_items})",
            values.len()
        ));
    }
    Ok(values)
}

fn required_string_array(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, String> {
    required_array(object, field, label, max_items)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            parse_string(
                value,
                &format!("{label}.{field}[{index}]"),
                max_chars,
                false,
                true,
            )
        })
        .collect()
}

fn require_uuidv7(label: &str, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let hyphens = [8, 13, 18, 23];
    let canonical = bytes.len() == 36
        && hyphens.iter().all(|index| bytes[*index] == b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            hyphens.contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b');
    if !canonical {
        return Err(format!(
            "{label} must be a canonical lowercase UUIDv7 string"
        ));
    }
    Ok(())
}

fn require_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} must be a canonical lowercase SHA-256 hex digest"));
    }
    Ok(())
}

fn require_revision(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_REVISION_CHARS
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.starts_with('0')
    {
        return Err(format!(
            "{label} must be a canonical positive decimal revision string"
        ));
    }
    Ok(())
}

fn require_unique_strings(label: &str, values: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if let Some(value) = values.iter().find(|value| !seen.insert(value.as_str())) {
        return Err(format!("{label} contains duplicate value `{value}`"));
    }
    Ok(())
}

fn parse_uuid_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<String, String> {
    let value = required_string(object, field, label, 36)?;
    require_uuidv7(&format!("{label}.{field}"), &value)?;
    Ok(value)
}

fn optional_uuid_string(
    object: &mut Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<String>, String> {
    let value = optional_string(object, field, label, 36)?;
    if let Some(value) = value.as_deref() {
        require_uuidv7(&format!("{label}.{field}"), value)?;
    }
    Ok(value)
}

fn parse_empty_request(label: &str, value: Value) -> Result<(), String> {
    let object = object(value, label)?;
    reject_unknown_fields(&object, &[], label)
}

fn parse_creation_target(value: Value, label: &str) -> Result<CreationTaskTarget, String> {
    let mut target = object(value, label)?;
    let kind = required_string(&mut target, "kind", label, 32)?;
    let parsed = match kind.as_str() {
        "canvas_node" => {
            reject_unknown_fields(&target, &["canvas_id", "node_id"], label)?;
            CreationTaskTarget::CanvasNode {
                canvas_id: parse_uuid_string(&mut target, "canvas_id", label)?,
                node_id: parse_uuid_string(&mut target, "node_id", label)?,
            }
        }
        "standalone_workbench" => {
            reject_unknown_fields(&target, &["workbench_kind"], label)?;
            let workbench_kind = required_string(&mut target, "workbench_kind", label, 16)?;
            let workbench_kind = match workbench_kind.as_str() {
                "image" => CreationWorkbenchKind::Image,
                "video" => CreationWorkbenchKind::Video,
                "audio" => CreationWorkbenchKind::Audio,
                _ => {
                    return Err(format!(
                        "{label}.workbench_kind must be image, video, or audio"
                    ));
                }
            };
            CreationTaskTarget::StandaloneWorkbench { workbench_kind }
        }
        "template_step" => {
            reject_unknown_fields(
                &target,
                &["template_id", "template_run_id", "template_step_id"],
                label,
            )?;
            CreationTaskTarget::TemplateStep {
                template_id: parse_uuid_string(&mut target, "template_id", label)?,
                template_run_id: parse_uuid_string(&mut target, "template_run_id", label)?,
                template_step_id: parse_uuid_string(&mut target, "template_step_id", label)?,
            }
        }
        _ => {
            return Err(format!(
                "{label}.kind must be canvas_node, standalone_workbench, or template_step"
            ));
        }
    };
    Ok(parsed)
}

fn parse_creation_text(value: Value) -> Result<CreationTextRequest, String> {
    const LABEL: &str = "creation.text input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["target", "prompt", "system", "max_tokens"], LABEL)?;
    let request = CreationTextRequest {
        target: parse_creation_target(required_value(&mut object, "target", LABEL)?, "target")?,
        prompt: required_untrimmed_string(&mut object, "prompt", LABEL, MAX_PROMPT_CHARS)?,
        system: object
            .remove("system")
            .map(|value| {
                parse_string(
                    value,
                    "creation.text input.system",
                    MAX_SYSTEM_CHARS,
                    true,
                    false,
                )
            })
            .transpose()?,
        max_tokens: match optional_u64(&mut object, "max_tokens", LABEL)? {
            Some(value) => u32::try_from(value)
                .ok()
                .filter(|value| (1..=131_072).contains(value))
                .ok_or_else(|| {
                    "creation.text input.max_tokens must be between 1 and 131072".to_owned()
                })?,
            None => 4_096,
        },
    };
    validate_creation_text(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn parse_creation_image(value: Value) -> Result<CreationImageRequest, String> {
    const LABEL: &str = "creation.image input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &["target", "prompt", "count", "size", "quality"],
        LABEL,
    )?;
    let request = CreationImageRequest {
        target: parse_creation_target(required_value(&mut object, "target", LABEL)?, "target")?,
        prompt: required_untrimmed_string(&mut object, "prompt", LABEL, MAX_PROMPT_CHARS)?,
        count: match optional_u64(&mut object, "count", LABEL)? {
            Some(value) => u32::try_from(value).map_err(|_| {
                "creation.image input.count must be a 32-bit integer".to_owned()
            })?,
            None => 1,
        },
        size: optional_string(&mut object, "size", LABEL, 128)?,
        quality: optional_string(&mut object, "quality", LABEL, 128)?,
    };
    validate_creation_image(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn parse_creation_image_edit(value: Value) -> Result<CreationImageEditRequest, String> {
    const LABEL: &str = "creation.image_edit input";
    let mut payload = object(value, LABEL)?;
    reject_unknown_fields(
        &payload,
        &["target", "prompt", "inputs", "count", "size"],
        LABEL,
    )?;
    let inputs = required_array(&mut payload, "inputs", LABEL, MAX_IMAGE_EDIT_INPUTS)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let item_label = format!("{LABEL}.inputs[{index}]");
            let mut item = object(value, &item_label)?;
            reject_unknown_fields(&item, &["asset_id", "role"], &item_label)?;
            let asset_id = parse_uuid_string(&mut item, "asset_id", &item_label)?;
            let role = match required_string(&mut item, "role", &item_label, 16)?.as_str() {
                "reference" => CreationImageInputRole::Reference,
                "mask" => CreationImageInputRole::Mask,
                _ => {
                    return Err(format!(
                        "{item_label}.role must be reference or mask"
                    ));
                }
            };
            Ok(CreationImageInput { asset_id, role })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let request = CreationImageEditRequest {
        target: parse_creation_target(required_value(&mut payload, "target", LABEL)?, "target")?,
        prompt: required_untrimmed_string(&mut payload, "prompt", LABEL, MAX_PROMPT_CHARS)?,
        inputs,
        count: match optional_u64(&mut payload, "count", LABEL)? {
            Some(value) => u32::try_from(value).map_err(|_| {
                "creation.image_edit input.count must be a 32-bit integer".to_owned()
            })?,
            None => 1,
        },
        size: optional_string(&mut payload, "size", LABEL, 128)?,
    };
    validate_creation_image_edit(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn parse_creation_video(value: Value) -> Result<CreationVideoRequest, String> {
    const LABEL: &str = "creation.video input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &[
            "target",
            "prompt",
            "seconds",
            "size",
            "first_frame_asset_id",
            "last_frame_asset_id",
        ],
        LABEL,
    )?;
    let request = CreationVideoRequest {
        target: parse_creation_target(required_value(&mut object, "target", LABEL)?, "target")?,
        prompt: required_untrimmed_string(&mut object, "prompt", LABEL, MAX_PROMPT_CHARS)?,
        seconds: optional_u64(&mut object, "seconds", LABEL)?
            .map(|value| {
                u32::try_from(value)
                    .ok()
                    .filter(|value| *value > 0 && *value <= 3_600)
                    .ok_or_else(|| {
                        "creation.video input.seconds must be between 1 and 3600".to_owned()
                    })
            })
            .transpose()?,
        size: optional_string(&mut object, "size", LABEL, 128)?,
        first_frame_asset_id: optional_uuid_string(
            &mut object,
            "first_frame_asset_id",
            LABEL,
        )?,
        last_frame_asset_id: optional_uuid_string(
            &mut object,
            "last_frame_asset_id",
            LABEL,
        )?,
    };
    validate_creation_video(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn parse_creation_audio(value: Value) -> Result<CreationAudioRequest, String> {
    const LABEL: &str = "creation.audio input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["target", "text", "voice", "format"], LABEL)?;
    let request = CreationAudioRequest {
        target: parse_creation_target(required_value(&mut object, "target", LABEL)?, "target")?,
        text: required_untrimmed_string(&mut object, "text", LABEL, MAX_PROMPT_CHARS)?,
        voice: optional_string(&mut object, "voice", LABEL, 256)?,
        format: optional_string(&mut object, "format", LABEL, 64)?,
    };
    validate_creation_audio(&request).map_err(|error| error.message)?;
    Ok(request)
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
            let mut ids = BTreeSet::new();
            let mut masks = 0;
            for input in &request.inputs {
                require_uuidv7("inputs[].asset_id", &input.asset_id)?;
                if !ids.insert(input.asset_id.as_str()) {
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

fn validate_creation_count(count: u32) -> Result<(), String> {
    if (1..=MAX_CREATION_RESULTS as u32).contains(&count) {
        Ok(())
    } else {
        Err(format!(
            "count must be between 1 and {MAX_CREATION_RESULTS}"
        ))
    }
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

fn parse_creation_status(value: &str) -> Result<CreationTaskStatus, String> {
    match value {
        "queued" => Ok(CreationTaskStatus::Queued),
        "running" => Ok(CreationTaskStatus::Running),
        "succeeded" => Ok(CreationTaskStatus::Succeeded),
        "failed" => Ok(CreationTaskStatus::Failed),
        "canceled" => Ok(CreationTaskStatus::Canceled),
        _ => Err("status must be queued, running, succeeded, failed, or canceled".to_owned()),
    }
}

fn parse_creation_outcome(
    value: Value,
) -> Result<(String, CreationTaskStatus, Vec<String>), String> {
    const LABEL: &str = "creation outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &["creation_task_id", "status", "result_asset_ids"],
        LABEL,
    )?;
    let task_id = parse_uuid_string(&mut object, "creation_task_id", LABEL)?;
    let status = parse_creation_status(&required_string(&mut object, "status", LABEL, 16)?)?;
    let result_asset_ids = required_string_array(
        &mut object,
        "result_asset_ids",
        LABEL,
        MAX_CREATION_RESULTS,
        36,
    )?;
    for asset_id in &result_asset_ids {
        require_uuidv7("creation outcome.result_asset_ids[]", asset_id)?;
    }
    require_unique_strings("creation outcome.result_asset_ids", &result_asset_ids)?;
    match status {
        CreationTaskStatus::Succeeded if result_asset_ids.is_empty() => {
            return Err("succeeded creation outcome requires result_asset_ids".to_owned());
        }
        CreationTaskStatus::Succeeded => {}
        _ if !result_asset_ids.is_empty() => {
            return Err(
                "non-succeeded creation outcome must not claim result_asset_ids".to_owned(),
            );
        }
        _ => {}
    }
    Ok((task_id, status, result_asset_ids))
}

fn creation_outcome_json(
    creation_task_id: String,
    status: CreationTaskStatus,
    result_asset_ids: Vec<String>,
) -> Value {
    json!({
        "creation_task_id": creation_task_id,
        "status": status.as_str(),
        "result_asset_ids": result_asset_ids,
    })
}

fn parse_canvas_node_type(value: &str, label: &str) -> Result<WorkshopCanvasNodeType, String> {
    match value {
        "image" => Ok(WorkshopCanvasNodeType::Image),
        "panorama" => Ok(WorkshopCanvasNodeType::Panorama),
        "text" => Ok(WorkshopCanvasNodeType::Text),
        "config" => Ok(WorkshopCanvasNodeType::Config),
        "video" => Ok(WorkshopCanvasNodeType::Video),
        "audio" => Ok(WorkshopCanvasNodeType::Audio),
        "director" => Ok(WorkshopCanvasNodeType::Director),
        "group" => Ok(WorkshopCanvasNodeType::Group),
        _ => Err(format!(
            "{label} must be image, panorama, text, config, video, audio, director, or group"
        )),
    }
}

fn parse_canvas_edit_operation(
    value: Value,
    index: usize,
) -> Result<WorkshopCanvasEditOperation, String> {
    let label = format!("workshop.canvas.edit input.operations[{index}]");
    let mut object = object(value, &label)?;
    let operation_type = required_string(&mut object, "type", &label, 32)?;
    let operation = match operation_type.as_str() {
        "add_node" => {
            reject_unknown_fields(
                &object,
                &[
                    "node_type",
                    "x",
                    "y",
                    "width",
                    "height",
                    "group_id",
                    "data",
                ],
                &label,
            )?;
            let node_type = parse_canvas_node_type(
                &required_string(&mut object, "node_type", &label, 32)?,
                &format!("{label}.node_type"),
            )?;
            let x = required_f64(&mut object, "x", &label)?;
            let y = required_f64(&mut object, "y", &label)?;
            let width = optional_f64(&mut object, "width", &label)?;
            let height = optional_f64(&mut object, "height", &label)?;
            let group_id = optional_uuid_string(&mut object, "group_id", &label)?;
            let data = required_value(&mut object, "data", &label)?;
            if !data.is_object() {
                return Err(format!("{label}.data must be a JSON object"));
            }
            validate_json_size(
                "workshop.canvas.edit",
                &format!("operations[{index}].data"),
                &data,
                MAX_CANVAS_VALUE_BYTES,
            )?;
            WorkshopCanvasEditOperation::AddNode {
                node_type,
                x,
                y,
                width,
                height,
                group_id,
                data: StrictJsonValue(data),
            }
        }
        "update_node_data" => {
            reject_unknown_fields(&object, &["node_id", "patch"], &label)?;
            let node_id = parse_uuid_string(&mut object, "node_id", &label)?;
            let patch = required_value(&mut object, "patch", &label)?;
            if !patch.is_object() {
                return Err(format!("{label}.patch must be a JSON object"));
            }
            validate_json_size(
                "workshop.canvas.edit",
                &format!("operations[{index}].patch"),
                &patch,
                MAX_CANVAS_VALUE_BYTES,
            )?;
            WorkshopCanvasEditOperation::UpdateNodeData {
                node_id,
                patch: StrictJsonValue(patch),
            }
        }
        "move_node" => {
            reject_unknown_fields(&object, &["node_id", "x", "y"], &label)?;
            WorkshopCanvasEditOperation::MoveNode {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
                x: required_f64(&mut object, "x", &label)?,
                y: required_f64(&mut object, "y", &label)?,
            }
        }
        "resize_node" => {
            reject_unknown_fields(&object, &["node_id", "width", "height"], &label)?;
            WorkshopCanvasEditOperation::ResizeNode {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
                width: required_f64(&mut object, "width", &label)?,
                height: required_f64(&mut object, "height", &label)?,
            }
        }
        "connect" => {
            reject_unknown_fields(
                &object,
                &[
                    "source_node_id",
                    "target_node_id",
                    "source_handle",
                    "target_handle",
                ],
                &label,
            )?;
            WorkshopCanvasEditOperation::Connect {
                source_node_id: parse_uuid_string(&mut object, "source_node_id", &label)?,
                target_node_id: parse_uuid_string(&mut object, "target_node_id", &label)?,
                source_handle: optional_string(&mut object, "source_handle", &label, 256)?,
                target_handle: optional_string(&mut object, "target_handle", &label, 256)?,
            }
        }
        "disconnect" => {
            reject_unknown_fields(&object, &["connection_id"], &label)?;
            WorkshopCanvasEditOperation::Disconnect {
                connection_id: parse_uuid_string(&mut object, "connection_id", &label)?,
            }
        }
        "delete_node" => {
            return Err(format!(
                "{label}.type delete_node requires explicit user confirmation and is not \
                 available to workshop.canvas.edit"
            ));
        }
        _ => {
            return Err(format!(
                "{label}.type is not a declared Canvas edit operation"
            ));
        }
    };
    Ok(operation)
}

fn parse_canvas_edit(value: Value) -> Result<WorkshopCanvasEditRequest, String> {
    const LABEL: &str = "workshop.canvas.edit input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["expected_revision", "operations"], LABEL)?;
    let expected_revision =
        required_string(&mut object, "expected_revision", LABEL, MAX_REVISION_CHARS)?;
    let operations = required_array(&mut object, "operations", LABEL, MAX_CANVAS_OPS)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_canvas_edit_operation(value, index))
        .collect::<Result<Vec<_>, _>>()?;
    let request = WorkshopCanvasEditRequest {
        expected_revision,
        operations,
    };
    validate_canvas_edit(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn validate_canvas_edit(request: &WorkshopCanvasEditRequest) -> Result<(), Wave3HostPortError> {
    let validate = || -> Result<(), String> {
        require_revision("expected_revision", &request.expected_revision)?;
        if request.operations.is_empty() || request.operations.len() > MAX_CANVAS_OPS {
            return Err(format!(
                "operations must contain 1 to {MAX_CANVAS_OPS} entries"
            ));
        }
        for (index, operation) in request.operations.iter().enumerate() {
            let label = format!("operations[{index}]");
            match operation {
                WorkshopCanvasEditOperation::AddNode {
                    x,
                    y,
                    width,
                    height,
                    group_id,
                    data,
                    ..
                } => {
                    if !x.is_finite() || !y.is_finite() {
                        return Err(format!("{label} coordinates must be finite"));
                    }
                    for (dimension, value) in [("width", width), ("height", height)] {
                        if value.is_some_and(|value| !value.is_finite() || value < 1.0) {
                            return Err(format!("{label}.{dimension} must be finite and at least 1"));
                        }
                    }
                    if let Some(group_id) = group_id {
                        require_uuidv7(&format!("{label}.group_id"), group_id)?;
                    }
                    if !data.0.is_object() {
                        return Err(format!("{label}.data must be an object"));
                    }
                    validate_json_size(
                        "workshop.canvas.edit",
                        &format!("{label}.data"),
                        &data.0,
                        MAX_CANVAS_VALUE_BYTES,
                    )?;
                }
                WorkshopCanvasEditOperation::UpdateNodeData { node_id, patch } => {
                    require_uuidv7(&format!("{label}.node_id"), node_id)?;
                    if !patch.0.is_object() {
                        return Err(format!("{label}.patch must be an object"));
                    }
                    validate_json_size(
                        "workshop.canvas.edit",
                        &format!("{label}.patch"),
                        &patch.0,
                        MAX_CANVAS_VALUE_BYTES,
                    )?;
                }
                WorkshopCanvasEditOperation::MoveNode { node_id, x, y } => {
                    require_uuidv7(&format!("{label}.node_id"), node_id)?;
                    if !x.is_finite() || !y.is_finite() {
                        return Err(format!("{label} coordinates must be finite"));
                    }
                }
                WorkshopCanvasEditOperation::ResizeNode {
                    node_id,
                    width,
                    height,
                } => {
                    require_uuidv7(&format!("{label}.node_id"), node_id)?;
                    if !width.is_finite()
                        || !height.is_finite()
                        || *width < 1.0
                        || *height < 1.0
                    {
                        return Err(format!(
                            "{label} dimensions must be finite and at least 1"
                        ));
                    }
                }
                WorkshopCanvasEditOperation::Connect {
                    source_node_id,
                    target_node_id,
                    source_handle,
                    target_handle,
                } => {
                    require_uuidv7(&format!("{label}.source_node_id"), source_node_id)?;
                    require_uuidv7(&format!("{label}.target_node_id"), target_node_id)?;
                    if source_node_id == target_node_id {
                        return Err(format!("{label} cannot connect a node to itself"));
                    }
                    validate_optional_short(
                        &format!("{label}.source_handle"),
                        source_handle.as_deref(),
                        256,
                    )?;
                    validate_optional_short(
                        &format!("{label}.target_handle"),
                        target_handle.as_deref(),
                        256,
                    )?;
                }
                WorkshopCanvasEditOperation::Disconnect { connection_id } => {
                    require_uuidv7(&format!("{label}.connection_id"), connection_id)?;
                }
            }
        }
        Ok(())
    };
    validate().map_err(Wave3HostPortError::invalid_request)
}

fn parse_canvas_read_outcome(value: Value) -> Result<WorkshopCanvasReadOutcome, String> {
    const LABEL: &str = "workshop.canvas.read outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &["canvas_id", "revision", "document_digest", "document"],
        LABEL,
    )?;
    let canvas_id = parse_uuid_string(&mut object, "canvas_id", LABEL)?;
    let revision = required_string(&mut object, "revision", LABEL, MAX_REVISION_CHARS)?;
    require_revision("workshop.canvas.read outcome.revision", &revision)?;
    let document_digest = required_string(&mut object, "document_digest", LABEL, 64)?;
    require_sha256(
        "workshop.canvas.read outcome.document_digest",
        &document_digest,
    )?;
    let document = required_value(&mut object, "document", LABEL)?;
    if !document.is_object() {
        return Err("workshop.canvas.read outcome.document must be an object".to_owned());
    }
    validate_json_size(
        "workshop.canvas.read",
        "outcome.document",
        &document,
        MAX_ACTION_OUTPUT_BYTES,
    )?;
    Ok(WorkshopCanvasReadOutcome {
        canvas_id,
        revision,
        document_digest,
        document: StrictJsonValue(document),
    })
}

fn parse_canvas_edit_outcome_operation(
    value: Value,
    index: usize,
) -> Result<WorkshopCanvasEditOperationOutcome, String> {
    let label = format!("workshop.canvas.edit outcome.operation_results[{index}]");
    let mut object = object(value, &label)?;
    let operation_type = required_string(&mut object, "type", &label, 32)?;
    let result = match operation_type.as_str() {
        "node_added" => {
            reject_unknown_fields(&object, &["node_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodeAdded {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
            }
        }
        "node_updated" => {
            reject_unknown_fields(&object, &["node_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodeUpdated {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
            }
        }
        "node_moved" => {
            reject_unknown_fields(&object, &["node_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodeMoved {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
            }
        }
        "node_resized" => {
            reject_unknown_fields(&object, &["node_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodeResized {
                node_id: parse_uuid_string(&mut object, "node_id", &label)?,
            }
        }
        "nodes_connected" => {
            reject_unknown_fields(&object, &["connection_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodesConnected {
                connection_id: parse_uuid_string(&mut object, "connection_id", &label)?,
            }
        }
        "nodes_disconnected" => {
            reject_unknown_fields(&object, &["connection_id"], &label)?;
            WorkshopCanvasEditOperationOutcome::NodesDisconnected {
                connection_id: parse_uuid_string(&mut object, "connection_id", &label)?,
            }
        }
        _ => return Err(format!("{label}.type is not a declared Canvas operation result")),
    };
    Ok(result)
}

fn parse_canvas_edit_outcome(value: Value) -> Result<WorkshopCanvasEditOutcome, String> {
    const LABEL: &str = "workshop.canvas.edit outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &[
            "canvas_id",
            "applied_revision",
            "replayed",
            "operation_results",
        ],
        LABEL,
    )?;
    let canvas_id = parse_uuid_string(&mut object, "canvas_id", LABEL)?;
    let applied_revision =
        required_string(&mut object, "applied_revision", LABEL, MAX_REVISION_CHARS)?;
    require_revision(
        "workshop.canvas.edit outcome.applied_revision",
        &applied_revision,
    )?;
    let replayed = required_bool(&mut object, "replayed", LABEL)?;
    let operation_results =
        required_array(&mut object, "operation_results", LABEL, MAX_CANVAS_OPS)?
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_canvas_edit_outcome_operation(value, index))
            .collect::<Result<Vec<_>, _>>()?;
    if operation_results.is_empty() {
        return Err("workshop.canvas.edit outcome.operation_results must not be empty".to_owned());
    }
    Ok(WorkshopCanvasEditOutcome {
        canvas_id,
        applied_revision,
        replayed,
        operation_results,
    })
}

fn canvas_edit_outcome_json(outcome: WorkshopCanvasEditOperationOutcome) -> Value {
    match outcome {
        WorkshopCanvasEditOperationOutcome::NodeAdded { node_id } => {
            json!({"type": "node_added", "node_id": node_id})
        }
        WorkshopCanvasEditOperationOutcome::NodeUpdated { node_id } => {
            json!({"type": "node_updated", "node_id": node_id})
        }
        WorkshopCanvasEditOperationOutcome::NodeMoved { node_id } => {
            json!({"type": "node_moved", "node_id": node_id})
        }
        WorkshopCanvasEditOperationOutcome::NodeResized { node_id } => {
            json!({"type": "node_resized", "node_id": node_id})
        }
        WorkshopCanvasEditOperationOutcome::NodesConnected { connection_id } => {
            json!({"type": "nodes_connected", "connection_id": connection_id})
        }
        WorkshopCanvasEditOperationOutcome::NodesDisconnected { connection_id } => {
            json!({"type": "nodes_disconnected", "connection_id": connection_id})
        }
    }
}

fn parse_asset_read(value: Value) -> Result<WorkshopAssetReadRequest, String> {
    const LABEL: &str = "workshop.asset.read input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["asset_id"], LABEL)?;
    let request = WorkshopAssetReadRequest {
        asset_id: parse_uuid_string(&mut object, "asset_id", LABEL)?,
    };
    validate_asset_read(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn parse_tags(value: Value, label: &str) -> Result<Vec<String>, String> {
    let values = value
        .as_array()
        .cloned()
        .ok_or_else(|| format!("{label} must be an array"))?;
    if values.len() > MAX_ASSET_TAGS {
        return Err(format!("{label} contains too many tags (max {MAX_ASSET_TAGS})"));
    }
    let tags = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            parse_string(
                value,
                &format!("{label}[{index}]"),
                MAX_ASSET_TAG_CHARS,
                false,
                true,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    require_unique_strings(label, &tags)?;
    Ok(tags)
}

fn parse_asset_write(value: Value) -> Result<WorkshopAssetWriteRequest, String> {
    const LABEL: &str = "workshop.asset.write input";
    let mut object = object(value, LABEL)?;
    let operation = required_string(&mut object, "operation", LABEL, 32)?;
    let request = match operation.as_str() {
        "create_text" => {
            reject_unknown_fields(
                &object,
                &[
                    "title",
                    "text_content",
                    "collection",
                    "tags",
                    "in_library",
                ],
                LABEL,
            )?;
            let title = required_string(&mut object, "title", LABEL, 1_000)?;
            let text_content =
                required_untrimmed_string(&mut object, "text_content", LABEL, MAX_ASSET_TEXT_BYTES)?;
            if text_content.as_bytes().len() > MAX_ASSET_TEXT_BYTES {
                return Err(format!(
                    "{LABEL}.text_content exceeds the {MAX_ASSET_TEXT_BYTES}-byte limit"
                ));
            }
            let collection = optional_clearable_string(&mut object, "collection", LABEL, 1_000)?;
            let tags = match object.remove("tags") {
                Some(value) => parse_tags(value, &format!("{LABEL}.tags"))?,
                None => Vec::new(),
            };
            let in_library = optional_bool(&mut object, "in_library", LABEL, true)?;
            WorkshopAssetWriteRequest::CreateText {
                title,
                text_content,
                collection,
                tags,
                in_library,
            }
        }
        "update_metadata" => {
            reject_unknown_fields(
                &object,
                &["asset_id", "title", "collection", "tags", "in_library"],
                LABEL,
            )?;
            let changes_requested = ["title", "collection", "tags", "in_library"]
                .iter()
                .any(|field| object.contains_key(*field));
            let asset_id = parse_uuid_string(&mut object, "asset_id", LABEL)?;
            let title = optional_string(&mut object, "title", LABEL, 1_000)?;
            let collection = optional_clearable_string(&mut object, "collection", LABEL, 1_000)?;
            let tags = object
                .remove("tags")
                .map(|value| parse_tags(value, &format!("{LABEL}.tags")))
                .transpose()?;
            let in_library = optional_nullable_bool(&mut object, "in_library", LABEL)?;
            if !changes_requested {
                return Err(
                    "workshop.asset.write update_metadata requires at least one changed field"
                        .to_owned(),
                );
            }
            WorkshopAssetWriteRequest::UpdateMetadata {
                asset_id,
                title,
                collection,
                tags,
                in_library,
            }
        }
        _ => {
            return Err(format!(
                "{LABEL}.operation must be create_text or update_metadata"
            ));
        }
    };
    validate_asset_write(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn validate_asset_read(request: &WorkshopAssetReadRequest) -> Result<(), Wave3HostPortError> {
    require_uuidv7("asset_id", &request.asset_id)
        .map_err(Wave3HostPortError::invalid_request)
}

fn validate_asset_write(request: &WorkshopAssetWriteRequest) -> Result<(), Wave3HostPortError> {
    let validate = || -> Result<(), String> {
        match request {
            WorkshopAssetWriteRequest::CreateText {
                title,
                text_content,
                collection,
                tags,
                ..
            } => {
                require_bounded_text("title", title, 1_000, false)?;
                if title.trim() != title {
                    return Err("title must be trimmed".to_owned());
                }
                if text_content.as_bytes().len() > MAX_ASSET_TEXT_BYTES {
                    return Err(format!(
                        "text_content exceeds the {MAX_ASSET_TEXT_BYTES}-byte limit"
                    ));
                }
                validate_collection(collection.as_deref())?;
                validate_tags(tags)
            }
            WorkshopAssetWriteRequest::UpdateMetadata {
                asset_id,
                title,
                collection,
                tags,
                in_library,
            } => {
                require_uuidv7("asset_id", asset_id)?;
                if title.is_none()
                    && collection.is_none()
                    && tags.is_none()
                    && in_library.is_none()
                {
                    return Err("update_metadata requires at least one changed field".to_owned());
                }
                if let Some(title) = title {
                    require_bounded_text("title", title, 1_000, false)?;
                    if title.trim() != title {
                        return Err("title must be trimmed".to_owned());
                    }
                }
                validate_collection(collection.as_deref())?;
                if let Some(tags) = tags {
                    validate_tags(tags)?;
                }
                Ok(())
            }
        }
    };
    validate().map_err(Wave3HostPortError::invalid_request)
}

fn validate_collection(collection: Option<&str>) -> Result<(), String> {
    if let Some(collection) = collection {
        require_bounded_text("collection", collection, 1_000, true)?;
        if collection.trim() != collection {
            return Err("collection must be trimmed".to_owned());
        }
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> Result<(), String> {
    if tags.len() > MAX_ASSET_TAGS {
        return Err(format!("tags exceeds {MAX_ASSET_TAGS} entries"));
    }
    require_unique_strings("tags", tags)?;
    for tag in tags {
        require_bounded_text("tags[]", tag, MAX_ASSET_TAG_CHARS, false)?;
        if tag.trim() != tag {
            return Err("tags[] must be trimmed".to_owned());
        }
    }
    Ok(())
}

fn parse_asset_record(value: Value, label: &str) -> Result<WorkshopAssetRecord, String> {
    let mut object = object(value, label)?;
    reject_unknown_fields(
        &object,
        &[
            "asset_id",
            "kind",
            "title",
            "collection",
            "tags",
            "mime",
            "byte_size",
            "in_library",
            "text_content",
        ],
        label,
    )?;
    let asset_id = parse_uuid_string(&mut object, "asset_id", label)?;
    let kind = required_string(&mut object, "kind", label, 32)?;
    let title = required_string(&mut object, "title", label, 1_000)?;
    let collection = optional_nullable_string(&mut object, "collection", label, 1_000, true)?;
    let tags = parse_tags(
        required_value(&mut object, "tags", label)?,
        &format!("{label}.tags"),
    )?;
    let mime = optional_nullable_string(&mut object, "mime", label, 256, false)?;
    let byte_size = match object.remove("byte_size") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| format!("{label}.byte_size must be a non-negative integer or null"))?,
        ),
    };
    let in_library = required_bool(&mut object, "in_library", label)?;
    let text_content =
        optional_nullable_string(&mut object, "text_content", label, MAX_ASSET_TEXT_BYTES, true)?;
    if text_content
        .as_ref()
        .is_some_and(|content| content.as_bytes().len() > MAX_ASSET_TEXT_BYTES)
    {
        return Err(format!(
            "{label}.text_content exceeds the {MAX_ASSET_TEXT_BYTES}-byte limit"
        ));
    }
    if kind == "text" {
        if text_content.is_none() || byte_size.is_some() {
            return Err(format!(
                "{label} text assets require text_content and no byte_size"
            ));
        }
    } else if text_content.is_some() || byte_size.is_none() {
        return Err(format!(
            "{label} binary assets require byte_size and no text_content"
        ));
    }
    Ok(WorkshopAssetRecord {
        asset_id,
        kind,
        title,
        collection,
        tags,
        mime,
        byte_size,
        in_library,
        text_content,
    })
}

fn parse_asset_outcome(value: Value) -> Result<WorkshopAssetRecord, String> {
    const LABEL: &str = "workshop asset outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["asset"], LABEL)?;
    parse_asset_record(required_value(&mut object, "asset", LABEL)?, "asset")
}

fn workshop_asset_json(asset: WorkshopAssetRecord) -> Value {
    json!({
        "asset_id": asset.asset_id,
        "kind": asset.kind,
        "title": asset.title,
        "collection": asset.collection,
        "tags": asset.tags,
        "mime": asset.mime,
        "byte_size": asset.byte_size,
        "in_library": asset.in_library,
        "text_content": asset.text_content,
    })
}

fn parse_template_input(
    value: Value,
    index: usize,
) -> Result<WorkshopTemplateInputValue, String> {
    let label = format!("workshop.template.run input.inputs[{index}]");
    let mut object = object(value, &label)?;
    let input_type = required_string(&mut object, "type", &label, 32)?;
    let variable_id = parse_uuid_string(&mut object, "variable_id", &label)?;
    let input = match input_type.as_str() {
        "text" => {
            reject_unknown_fields(&object, &["value"], &label)?;
            WorkshopTemplateInputValue::Text {
                variable_id,
                value: required_untrimmed_string(
                    &mut object,
                    "value",
                    &label,
                    MAX_TEMPLATE_TEXT_CHARS,
                )?,
            }
        }
        "multiline-text" => {
            reject_unknown_fields(&object, &["value"], &label)?;
            WorkshopTemplateInputValue::MultilineText {
                variable_id,
                value: required_untrimmed_string(
                    &mut object,
                    "value",
                    &label,
                    MAX_TEMPLATE_TEXT_CHARS,
                )?,
            }
        }
        "number" => {
            reject_unknown_fields(&object, &["value"], &label)?;
            WorkshopTemplateInputValue::Number {
                variable_id,
                value: required_f64(&mut object, "value", &label)?,
            }
        }
        "boolean" => {
            reject_unknown_fields(&object, &["value"], &label)?;
            WorkshopTemplateInputValue::Boolean {
                variable_id,
                value: required_bool(&mut object, "value", &label)?,
            }
        }
        "choice" => {
            reject_unknown_fields(&object, &["value"], &label)?;
            WorkshopTemplateInputValue::Choice {
                variable_id,
                value: required_string(
                    &mut object,
                    "value",
                    &label,
                    MAX_TEMPLATE_TEXT_CHARS,
                )?,
            }
        }
        "image" => {
            reject_unknown_fields(&object, &["asset_id"], &label)?;
            WorkshopTemplateInputValue::Image {
                variable_id,
                asset_id: optional_uuid_string(&mut object, "asset_id", &label)?,
            }
        }
        "image-series" => {
            reject_unknown_fields(&object, &["asset_ids"], &label)?;
            let asset_ids =
                required_string_array(&mut object, "asset_ids", &label, 100, 36)?;
            for asset_id in &asset_ids {
                require_uuidv7(&format!("{label}.asset_ids[]"), asset_id)?;
            }
            require_unique_strings(&format!("{label}.asset_ids"), &asset_ids)?;
            WorkshopTemplateInputValue::ImageSeries {
                variable_id,
                asset_ids,
            }
        }
        _ => return Err(format!("{label}.type is not a declared template input type")),
    };
    Ok(input)
}

fn parse_template_run(value: Value) -> Result<WorkshopTemplateRunRequest, String> {
    const LABEL: &str = "workshop.template.run input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &[
            "template_run_id",
            "template_id",
            "template_revision",
            "inputs",
            "reference_asset_ids",
        ],
        LABEL,
    )?;
    let template_run_id = parse_uuid_string(&mut object, "template_run_id", LABEL)?;
    let template_id = parse_uuid_string(&mut object, "template_id", LABEL)?;
    let template_revision = required_i64(&mut object, "template_revision", LABEL)?;
    let inputs = required_array(&mut object, "inputs", LABEL, MAX_TEMPLATE_INPUTS)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_template_input(value, index))
        .collect::<Result<Vec<_>, _>>()?;
    let reference_asset_ids = required_string_array(
        &mut object,
        "reference_asset_ids",
        LABEL,
        MAX_TEMPLATE_REFERENCES,
        36,
    )?;
    for asset_id in &reference_asset_ids {
        require_uuidv7("workshop.template.run input.reference_asset_ids[]", asset_id)?;
    }
    let request = WorkshopTemplateRunRequest {
        template_run_id,
        template_id,
        template_revision,
        inputs,
        reference_asset_ids,
    };
    validate_template_run(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn template_input_variable_id(input: &WorkshopTemplateInputValue) -> &str {
    match input {
        WorkshopTemplateInputValue::Text { variable_id, .. }
        | WorkshopTemplateInputValue::MultilineText { variable_id, .. }
        | WorkshopTemplateInputValue::Number { variable_id, .. }
        | WorkshopTemplateInputValue::Boolean { variable_id, .. }
        | WorkshopTemplateInputValue::Choice { variable_id, .. }
        | WorkshopTemplateInputValue::Image { variable_id, .. }
        | WorkshopTemplateInputValue::ImageSeries { variable_id, .. } => variable_id,
    }
}

fn validate_template_run(
    request: &WorkshopTemplateRunRequest,
) -> Result<(), Wave3HostPortError> {
    let validate = || -> Result<(), String> {
        require_uuidv7("template_run_id", &request.template_run_id)?;
        require_uuidv7("template_id", &request.template_id)?;
        if request.template_revision < 1 {
            return Err("template_revision must be positive".to_owned());
        }
        if request.inputs.len() > MAX_TEMPLATE_INPUTS {
            return Err(format!("inputs exceeds {MAX_TEMPLATE_INPUTS} entries"));
        }
        if request.reference_asset_ids.len() > MAX_TEMPLATE_REFERENCES {
            return Err(format!(
                "reference_asset_ids exceeds {MAX_TEMPLATE_REFERENCES} entries"
            ));
        }
        let variable_ids = request
            .inputs
            .iter()
            .map(template_input_variable_id)
            .collect::<Vec<_>>();
        let mut seen_variables = BTreeSet::new();
        for variable_id in variable_ids {
            require_uuidv7("inputs[].variable_id", variable_id)?;
            if !seen_variables.insert(variable_id) {
                return Err(format!("duplicate template variable_id {variable_id}"));
            }
        }
        for input in &request.inputs {
            match input {
                WorkshopTemplateInputValue::Text { value, .. }
                | WorkshopTemplateInputValue::MultilineText { value, .. }
                | WorkshopTemplateInputValue::Choice { value, .. } => {
                    require_bounded_text(
                        "inputs[].value",
                        value,
                        MAX_TEMPLATE_TEXT_CHARS,
                        false,
                    )?;
                }
                WorkshopTemplateInputValue::Number { value, .. } if !value.is_finite() => {
                    return Err("inputs[].value must be finite".to_owned());
                }
                WorkshopTemplateInputValue::Image {
                    asset_id: Some(asset_id),
                    ..
                } => require_uuidv7("inputs[].asset_id", asset_id)?,
                WorkshopTemplateInputValue::ImageSeries { asset_ids, .. } => {
                    if asset_ids.len() > MAX_TEMPLATE_REFERENCES {
                        return Err("inputs[].asset_ids contains too many entries".to_owned());
                    }
                    require_unique_strings("inputs[].asset_ids", asset_ids)?;
                    for asset_id in asset_ids {
                        require_uuidv7("inputs[].asset_ids[]", asset_id)?;
                    }
                }
                _ => {}
            }
        }
        require_unique_strings("reference_asset_ids", &request.reference_asset_ids)?;
        for asset_id in &request.reference_asset_ids {
            require_uuidv7("reference_asset_ids[]", asset_id)?;
        }
        Ok(())
    };
    validate().map_err(Wave3HostPortError::invalid_request)
}

fn parse_template_status(value: &str) -> Result<WorkshopTemplateRunStatus, String> {
    match value {
        "requested" => Ok(WorkshopTemplateRunStatus::Requested),
        "awaiting-review" => Ok(WorkshopTemplateRunStatus::AwaitingReview),
        "queued" => Ok(WorkshopTemplateRunStatus::Queued),
        "running" => Ok(WorkshopTemplateRunStatus::Running),
        "succeeded" => Ok(WorkshopTemplateRunStatus::Succeeded),
        "failed" => Ok(WorkshopTemplateRunStatus::Failed),
        "cancelled" => Ok(WorkshopTemplateRunStatus::Cancelled),
        _ => Err("template run status is not canonical".to_owned()),
    }
}

fn parse_template_run_outcome(value: Value) -> Result<WorkshopTemplateRunOutcome, String> {
    const LABEL: &str = "workshop.template.run outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &[
            "template_run_id",
            "template_id",
            "revision",
            "status",
            "task_ids",
            "result_asset_ids",
        ],
        LABEL,
    )?;
    let template_run_id = parse_uuid_string(&mut object, "template_run_id", LABEL)?;
    let template_id = parse_uuid_string(&mut object, "template_id", LABEL)?;
    let revision = required_u64(&mut object, "revision", LABEL)?;
    if revision == 0 {
        return Err("workshop.template.run outcome.revision must be positive".to_owned());
    }
    let status = parse_template_status(&required_string(&mut object, "status", LABEL, 32)?)?;
    let task_ids =
        required_string_array(&mut object, "task_ids", LABEL, MAX_TEMPLATE_RESULT_IDS, 36)?;
    let result_asset_ids = required_string_array(
        &mut object,
        "result_asset_ids",
        LABEL,
        MAX_TEMPLATE_RESULT_IDS,
        36,
    )?;
    for (field, ids) in [
        ("task_ids", task_ids.as_slice()),
        ("result_asset_ids", result_asset_ids.as_slice()),
    ] {
        require_unique_strings(&format!("{LABEL}.{field}"), ids)?;
        for id in ids {
            require_uuidv7(&format!("{LABEL}.{field}[]"), id)?;
        }
    }
    Ok(WorkshopTemplateRunOutcome {
        template_run_id,
        template_id,
        revision,
        status,
        task_ids,
        result_asset_ids,
    })
}

fn parse_office_document_type(value: &str, label: &str) -> Result<OfficeDocumentType, String> {
    match value {
        "word" => Ok(OfficeDocumentType::Word),
        "excel" => Ok(OfficeDocumentType::Excel),
        "ppt" => Ok(OfficeDocumentType::Ppt),
        _ => Err(format!("{label} must be word, excel, or ppt")),
    }
}

fn parse_office_preview(value: Value) -> Result<OfficePreviewRequest, String> {
    const LABEL: &str = "office.preview input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["asset_id", "document_type"], LABEL)?;
    let request = OfficePreviewRequest {
        asset_id: parse_uuid_string(&mut object, "asset_id", LABEL)?,
        document_type: parse_office_document_type(
            &required_string(&mut object, "document_type", LABEL, 16)?,
            "office.preview input.document_type",
        )?,
    };
    validate_office_preview(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn validate_office_preview(request: &OfficePreviewRequest) -> Result<(), Wave3HostPortError> {
    require_uuidv7("asset_id", &request.asset_id)
        .map_err(Wave3HostPortError::invalid_request)
}

fn parse_office_preview_outcome(value: Value) -> Result<OfficePreviewOutcome, String> {
    const LABEL: &str = "office.preview outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &["asset_id", "document_type", "preview_url", "capability"],
        LABEL,
    )?;
    let asset_id = parse_uuid_string(&mut object, "asset_id", LABEL)?;
    let document_type = parse_office_document_type(
        &required_string(&mut object, "document_type", LABEL, 16)?,
        "office.preview outcome.document_type",
    )?;
    let preview_url = required_string(&mut object, "preview_url", LABEL, MAX_URL_CHARS)?;
    if !preview_url.starts_with("/api/") || !preview_url.ends_with('/') {
        return Err(
            "office.preview outcome.preview_url must be a root-relative /api/ URL ending in /"
                .to_owned(),
        );
    }
    let capability = required_string(&mut object, "capability", LABEL, 64)?;
    require_sha256("office.preview outcome.capability", &capability)?;
    Ok(OfficePreviewOutcome {
        asset_id,
        document_type,
        preview_url,
        capability,
    })
}

fn parse_miniapp_edit(value: Value) -> Result<MiniAppEditRequest, String> {
    const LABEL: &str = "miniapp.edit input";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["name", "description", "icon", "html"], LABEL)?;
    let request = MiniAppEditRequest {
        name: optional_string(&mut object, "name", LABEL, MAX_MINIAPP_NAME_CHARS)?,
        description: optional_clearable_string(
            &mut object,
            "description",
            LABEL,
            MAX_MINIAPP_DESCRIPTION_CHARS,
        )?,
        icon: optional_clearable_string(&mut object, "icon", LABEL, MAX_MINIAPP_ICON_CHARS)?,
        html: object
            .remove("html")
            .map(|value| {
                let html = parse_string(
                    value,
                    "miniapp.edit input.html",
                    MAX_MINIAPP_HTML_BYTES,
                    false,
                    false,
                )?;
                if html.as_bytes().len() > MAX_MINIAPP_HTML_BYTES {
                    return Err(format!(
                        "miniapp.edit input.html exceeds the {MAX_MINIAPP_HTML_BYTES}-byte limit"
                    ));
                }
                Ok(html)
            })
            .transpose()?,
    };
    validate_miniapp_edit(&request).map_err(|error| error.message)?;
    Ok(request)
}

fn validate_miniapp_edit(request: &MiniAppEditRequest) -> Result<(), Wave3HostPortError> {
    let validate = || -> Result<(), String> {
        if request.name.is_none()
            && request.description.is_none()
            && request.icon.is_none()
            && request.html.is_none()
        {
            return Err("miniapp.edit requires at least one changed field".to_owned());
        }
        if let Some(name) = request.name.as_deref() {
            require_bounded_text("name", name, MAX_MINIAPP_NAME_CHARS, false)?;
            if name.trim() != name {
                return Err("name must be trimmed".to_owned());
            }
        }
        if let Some(description) = request.description.as_deref() {
            require_bounded_text(
                "description",
                description,
                MAX_MINIAPP_DESCRIPTION_CHARS,
                true,
            )?;
            if description.trim() != description {
                return Err("description must be trimmed".to_owned());
            }
        }
        if let Some(icon) = request.icon.as_deref() {
            require_bounded_text("icon", icon, MAX_MINIAPP_ICON_CHARS, true)?;
            if icon.trim() != icon {
                return Err("icon must be trimmed".to_owned());
            }
        }
        if let Some(html) = request.html.as_deref() {
            if html.trim().is_empty() {
                return Err("html must not be blank".to_owned());
            }
            if html.as_bytes().len() > MAX_MINIAPP_HTML_BYTES {
                return Err(format!(
                    "html exceeds the {MAX_MINIAPP_HTML_BYTES}-byte limit"
                ));
            }
        }
        Ok(())
    };
    validate().map_err(Wave3HostPortError::invalid_request)
}

fn parse_miniapp_record(value: Value, label: &str) -> Result<MiniAppRecord, String> {
    let mut object = object(value, label)?;
    reject_unknown_fields(
        &object,
        &[
            "miniapp_id",
            "name",
            "description",
            "icon",
            "source_conversation_id",
            "html_size",
            "published_at",
            "has_unpublished_changes",
            "created_at",
            "updated_at",
        ],
        label,
    )?;
    let miniapp_id = parse_uuid_string(&mut object, "miniapp_id", label)?;
    let name = required_string(&mut object, "name", label, MAX_MINIAPP_NAME_CHARS)?;
    let description = parse_string(
        required_value(&mut object, "description", label)?,
        &format!("{label}.description"),
        MAX_MINIAPP_DESCRIPTION_CHARS,
        true,
        true,
    )?;
    let icon =
        optional_nullable_string(&mut object, "icon", label, MAX_MINIAPP_ICON_CHARS, false)?;
    let source_conversation_id =
        optional_nullable_string(&mut object, "source_conversation_id", label, 36, false)?;
    if let Some(source_conversation_id) = source_conversation_id.as_deref() {
        require_uuidv7(
            &format!("{label}.source_conversation_id"),
            source_conversation_id,
        )?;
    }
    let html_size = required_u64(&mut object, "html_size", label)?;
    if html_size == 0 || html_size > MAX_MINIAPP_HTML_BYTES as u64 {
        return Err(format!(
            "{label}.html_size must be between 1 and {MAX_MINIAPP_HTML_BYTES}"
        ));
    }
    let published_at = optional_nullable_i64(&mut object, "published_at", label)?;
    let has_unpublished_changes =
        required_bool(&mut object, "has_unpublished_changes", label)?;
    let created_at = required_i64(&mut object, "created_at", label)?;
    let updated_at = required_i64(&mut object, "updated_at", label)?;
    if created_at < 0 || updated_at < created_at || published_at.is_some_and(|value| value < 0) {
        return Err(format!("{label} timestamps are not canonical"));
    }
    Ok(MiniAppRecord {
        miniapp_id,
        name,
        description,
        icon,
        source_conversation_id,
        html_size,
        published_at,
        has_unpublished_changes,
        created_at,
        updated_at,
    })
}

fn parse_miniapp_outcome(value: Value) -> Result<MiniAppRecord, String> {
    const LABEL: &str = "miniapp outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(&object, &["app"], LABEL)?;
    parse_miniapp_record(required_value(&mut object, "app", LABEL)?, "app")
}

fn parse_miniapp_serve_outcome(value: Value) -> Result<MiniAppServeOutcome, String> {
    const LABEL: &str = "miniapp.serve outcome";
    let mut object = object(value, LABEL)?;
    reject_unknown_fields(
        &object,
        &["miniapp_id", "serve_url", "published_at"],
        LABEL,
    )?;
    let miniapp_id = parse_uuid_string(&mut object, "miniapp_id", LABEL)?;
    let serve_url = required_string(&mut object, "serve_url", LABEL, MAX_URL_CHARS)?;
    let expected_suffix = format!("/api/miniapps/{miniapp_id}/serve");
    if !serve_url.ends_with(&expected_suffix)
        || !(serve_url == expected_suffix || serve_url.starts_with("http://localhost:"))
    {
        return Err(format!(
            "miniapp.serve outcome.serve_url must name the bound canonical serve route {expected_suffix}"
        ));
    }
    let published_at = required_i64(&mut object, "published_at", LABEL)?;
    if published_at < 0 {
        return Err("miniapp.serve outcome.published_at must be non-negative".to_owned());
    }
    Ok(MiniAppServeOutcome {
        miniapp_id,
        serve_url,
        published_at,
    })
}

fn miniapp_record_json(app: MiniAppRecord) -> Value {
    json!({
        "miniapp_id": app.miniapp_id,
        "name": app.name,
        "description": app.description,
        "icon": app.icon,
        "source_conversation_id": app.source_conversation_id,
        "html_size": app.html_size,
        "published_at": app.published_at,
        "has_unpublished_changes": app.has_unpublished_changes,
        "created_at": app.created_at,
        "updated_at": app.updated_at,
    })
}

fn validate_outcome_resource_binding(
    outcome: &Wave3CapabilityOutcome,
    bindings: &[TypedResourceBinding],
) -> Result<(), Wave3HostPortError> {
    let expected = match outcome {
        Wave3CapabilityOutcome::WorkshopCanvasRead(outcome) => {
            Some((CANVAS_RESOURCE_KIND, outcome.canvas_id.as_str()))
        }
        Wave3CapabilityOutcome::WorkshopCanvasEdit(outcome) => {
            Some((CANVAS_RESOURCE_KIND, outcome.canvas_id.as_str()))
        }
        Wave3CapabilityOutcome::MiniAppRead(outcome) => {
            Some((MINIAPP_RESOURCE_KIND, outcome.app.miniapp_id.as_str()))
        }
        Wave3CapabilityOutcome::MiniAppEdit(outcome) => {
            Some((MINIAPP_RESOURCE_KIND, outcome.app.miniapp_id.as_str()))
        }
        Wave3CapabilityOutcome::MiniAppPublish(outcome) => {
            Some((MINIAPP_RESOURCE_KIND, outcome.app.miniapp_id.as_str()))
        }
        Wave3CapabilityOutcome::MiniAppServe(outcome) => {
            Some((MINIAPP_RESOURCE_KIND, outcome.miniapp_id.as_str()))
        }
        _ => None,
    };
    let Some((resource_kind, resource_id)) = expected else {
        return Ok(());
    };
    let binding = bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == resource_kind)
        .ok_or_else(|| {
            Wave3HostPortError::invalid_response(format!(
                "{} outcome has no {resource_kind} binding",
                outcome.capability_id().as_ref()
            ))
        })?;
    if binding.resource_id.as_ref() != resource_id {
        return Err(Wave3HostPortError::invalid_response(format!(
            "{} outcome resource {} does not match bound resource {}",
            outcome.capability_id().as_ref(),
            resource_id,
            binding.resource_id.as_ref()
        )));
    }
    Ok(())
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
                KernelError::CapabilityExecution {
                    reason: error.to_string(),
                }
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

    const ID_1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const ID_2: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const ID_3: &str = "0190f5fe-7c00-7a00-8000-000000000003";
    const ID_4: &str = "0190f5fe-7c00-7a00-8000-000000000004";

    fn action_contract_is_blocked(capability_id: &str) -> bool {
        matches!(
            capability_id,
            "workshop.director"
                | "office.document.edit"
                | "office.sheet.edit"
                | "office.slides.edit"
        )
    }

    fn sample_input(capability_id: &str) -> StrictJsonValue {
        let canvas_target = || {
            json!({
                "kind": "canvas_node",
                "canvas_id": ID_1,
                "node_id": ID_2
            })
        };
        let value = match capability_id {
            "creation.text" => json!({
                "target": canvas_target(),
                "prompt": "Draft a caption",
                "max_tokens": 512
            }),
            "creation.image" => json!({
                "target": {
                    "kind": "standalone_workbench",
                    "workbench_kind": "image"
                },
                "prompt": "A precise product photograph",
                "count": 1,
                "size": "1024x1024"
            }),
            "creation.image_edit" => json!({
                "target": canvas_target(),
                "prompt": "Replace the background",
                "inputs": [{"asset_id": ID_3, "role": "reference"}],
                "count": 1
            }),
            "creation.video" => json!({
                "target": {
                    "kind": "standalone_workbench",
                    "workbench_kind": "video"
                },
                "prompt": "A slow camera orbit",
                "seconds": 8,
                "first_frame_asset_id": ID_3
            }),
            "creation.audio" => json!({
                "target": {
                    "kind": "standalone_workbench",
                    "workbench_kind": "audio"
                },
                "text": "Welcome to NomiFun.",
                "voice": "alloy",
                "format": "mp3"
            }),
            "workshop.canvas.read" | "miniapp.read" | "miniapp.publish" | "miniapp.serve" => {
                json!({})
            }
            "workshop.canvas.edit" => json!({
                "expected_revision": "1",
                "operations": [{
                    "type": "move_node",
                    "node_id": ID_2,
                    "x": 12.5,
                    "y": 24.0
                }]
            }),
            "workshop.asset.read" => json!({"asset_id": ID_3}),
            "workshop.asset.write" => json!({
                "operation": "create_text",
                "title": "Caption",
                "text_content": "A bounded text asset.",
                "tags": ["draft"],
                "in_library": true
            }),
            "workshop.template.run" => json!({
                "template_run_id": ID_1,
                "template_id": ID_2,
                "template_revision": 1,
                "inputs": [{
                    "type": "text",
                    "variable_id": ID_3,
                    "value": "Aurora"
                }],
                "reference_asset_ids": [ID_4]
            }),
            "workshop.director"
            | "office.document.edit"
            | "office.sheet.edit"
            | "office.slides.edit" => json!({}),
            "office.preview" => json!({
                "asset_id": ID_3,
                "document_type": "word"
            }),
            "miniapp.edit" => json!({"name": "Timer"}),
            other => panic!("missing sample input for {other}"),
        };
        StrictJsonValue(value)
    }

    fn sample_output(capability_id: &str) -> StrictJsonValue {
        let app = || {
            json!({
                "miniapp_id": ID_1,
                "name": "Timer",
                "description": "A focused timer",
                "icon": null,
                "source_conversation_id": null,
                "html_size": 1024,
                "published_at": 1_700_000_000_000i64,
                "has_unpublished_changes": false,
                "created_at": 1_699_999_000_000i64,
                "updated_at": 1_700_000_000_000i64
            })
        };
        let value = match capability_id {
            "creation.text"
            | "creation.image"
            | "creation.image_edit"
            | "creation.video"
            | "creation.audio" => json!({
                "creation_task_id": ID_1,
                "status": "queued",
                "result_asset_ids": []
            }),
            "workshop.canvas.read" => json!({
                "canvas_id": ID_1,
                "revision": "1",
                "document_digest": "a".repeat(64),
                "document": {"schema": "nomifun.creative-studio/v1"}
            }),
            "workshop.canvas.edit" => json!({
                "canvas_id": ID_1,
                "applied_revision": "2",
                "replayed": false,
                "operation_results": [{"type": "node_moved", "node_id": ID_2}]
            }),
            "workshop.asset.read" | "workshop.asset.write" => json!({
                "asset": {
                    "asset_id": ID_3,
                    "kind": "text",
                    "title": "Caption",
                    "collection": null,
                    "tags": ["draft"],
                    "mime": null,
                    "byte_size": null,
                    "in_library": true,
                    "text_content": "A bounded text asset."
                }
            }),
            "workshop.template.run" => json!({
                "template_run_id": ID_1,
                "template_id": ID_2,
                "revision": 1,
                "status": "requested",
                "task_ids": [],
                "result_asset_ids": []
            }),
            "workshop.director"
            | "office.document.edit"
            | "office.sheet.edit"
            | "office.slides.edit" => json!({}),
            "office.preview" => json!({
                "asset_id": ID_3,
                "document_type": "word",
                "preview_url": format!("/api/office-watch-proxy/{}/", "b".repeat(64)),
                "capability": "b".repeat(64)
            }),
            "miniapp.read" | "miniapp.edit" | "miniapp.publish" => json!({"app": app()}),
            "miniapp.serve" => json!({
                "miniapp_id": ID_1,
                "serve_url": format!("/api/miniapps/{ID_1}/serve"),
                "published_at": 1_700_000_000_000i64
            }),
            other => panic!("missing sample output for {other}"),
        };
        StrictJsonValue(value)
    }

    fn bindings_for(capability_id: &str, owner_id: &str) -> TypedResourceBindings {
        let expected_kinds =
            required_resource_kinds(capability_id).expect("known Wave 3 capability");
        canonical_resource_bindings(owner_id)
            .into_iter()
            .filter(|binding| expected_kinds.contains(&binding.resource_kind))
            .map(|mut binding| {
                if binding.resource_kind.as_ref() == CANVAS_RESOURCE_KIND
                    || binding.resource_kind.as_ref() == MINIAPP_RESOURCE_KIND
                {
                    binding.resource_id = ResourceId::from(ID_1);
                }
                binding
            })
            .collect()
    }

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
            let result = operation_from_input(&capability_id, sample_input(capability.id));
            if action_contract_is_blocked(capability.id) {
                let error = result.expect_err("unfrozen action must fail closed");
                assert!(
                    error.to_string().contains(WAVE3_CONTRACT_NOT_FROZEN),
                    "{} returned the wrong blocker: {error}",
                    capability.id
                );
            } else {
                let operation = result.unwrap_or_else(|error| {
                    panic!("{} must have a typed host operation: {error}", capability.id)
                });
                assert_eq!(operation.capability_id(), capability_id);
            }
        }
    }

    #[test]
    fn operation_mapping_and_resource_requirements_match_the_frozen_inventory() {
        for capability in PACKAGE_SPECS
            .iter()
            .flat_map(|package| package.capabilities.iter())
        {
            let capability_id = CapabilityId::from(capability.id);
            let bindings = bindings_for(capability.id, "wave3-test-owner");
            let result = operation_from_input(&capability_id, sample_input(capability.id));
            if action_contract_is_blocked(capability.id) {
                assert!(
                    result
                        .expect_err("unfrozen action must reject")
                        .to_string()
                        .contains(WAVE3_CONTRACT_NOT_FROZEN)
                );
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
                continue;
            }
            let operation = result.expect("active Wave 3 capability has a typed operation");
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
                        Wave3CapabilityOperation::WorkshopCanvasRead(_)
                    ));
                }
                "workshop.canvas.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopCanvasEdit(_)
                    ));
                }
                "workshop.asset.read" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopAssetRead(_)
                    ));
                }
                "workshop.asset.write" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopAssetWrite(_)
                    ));
                }
                "workshop.template.run" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::WorkshopTemplateRun(_)
                    ));
                }
                "office.preview" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::OfficePreview(_)
                    ));
                }
                "miniapp.read" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppRead(_)
                    ));
                }
                "miniapp.edit" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppEdit(_)
                    ));
                }
                "miniapp.publish" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppPublish(_)
                    ));
                }
                "miniapp.serve" => {
                    assert!(matches!(
                        operation,
                        Wave3CapabilityOperation::MiniAppServe(_)
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
    fn action_specific_schemas_and_unknown_fields_fail_closed() {
        for capability_id in ALL_CAPABILITY_IDS {
            let schema = action_input_schema(capability_id);
            if action_contract_is_blocked(capability_id) {
                assert_eq!(
                    schema.get("not"),
                    Some(&json!({})),
                    "{capability_id} must publish an impossible schema until its owner contract is frozen"
                );
                let error = operation_from_input(
                    &CapabilityId::from(capability_id),
                    sample_input(capability_id),
                )
                .expect_err("blocked action must reject");
                assert!(error.to_string().contains(WAVE3_CONTRACT_NOT_FROZEN));
                continue;
            }

            assert_ne!(
                schema,
                json!({"type": "object", "additionalProperties": true}),
                "{capability_id} must not retain the generic object schema"
            );
            let mut input = sample_input(capability_id).0;
            input
                .as_object_mut()
                .expect("sample input is an object")
                .insert("unknown_field".to_owned(), json!(true));
            let error = operation_from_input(
                &CapabilityId::from(capability_id),
                StrictJsonValue(input),
            )
            .expect_err("unknown top-level fields must reject");
            assert!(
                error.to_string().contains("unknown field `unknown_field`"),
                "{capability_id} returned the wrong unknown-field error: {error}"
            );
        }
    }

    #[test]
    fn representative_field_and_size_limits_reject_before_owner_dispatch() {
        let mut oversized_prompt = sample_input("creation.text").0;
        oversized_prompt["prompt"] = json!("x".repeat(MAX_PROMPT_CHARS + 1));
        let error = operation_from_input(
            &CapabilityId::from("creation.text"),
            StrictJsonValue(oversized_prompt),
        )
        .expect_err("oversized prompt must reject");
        assert!(error.to_string().contains("65536"));

        let mut excessive_inputs = sample_input("creation.image_edit").0;
        excessive_inputs["inputs"] = Value::Array(
            (0..=MAX_IMAGE_EDIT_INPUTS)
                .map(|index| {
                    json!({
                        "asset_id": format!("0190f5fe-7c00-7a00-8000-{index:012x}"),
                        "role": "reference"
                    })
                })
                .collect(),
        );
        let error = operation_from_input(
            &CapabilityId::from("creation.image_edit"),
            StrictJsonValue(excessive_inputs),
        )
        .expect_err("excessive image input count must reject");
        assert!(error.to_string().contains("max 8"));

        let mut empty_miniapp_edit = sample_input("miniapp.edit").0;
        empty_miniapp_edit
            .as_object_mut()
            .expect("object")
            .clear();
        let error = operation_from_input(
            &CapabilityId::from("miniapp.edit"),
            StrictJsonValue(empty_miniapp_edit),
        )
        .expect_err("empty edit must reject");
        assert!(error.to_string().contains("at least one changed field"));
    }

    #[test]
    fn all_outcomes_are_typed_canonical_or_explicitly_blocked() {
        for capability_id in ALL_CAPABILITY_IDS {
            let capability_id = CapabilityId::from(capability_id);
            let result = outcome_from_output(
                &capability_id,
                sample_output(capability_id.as_ref()),
            );
            if action_contract_is_blocked(capability_id.as_ref()) {
                let error = result.expect_err("blocked action cannot report success");
                assert_eq!(error.code, WAVE3_CONTRACT_NOT_FROZEN);
                continue;
            }

            let outcome = result.unwrap_or_else(|error| {
                panic!(
                    "{} sample outcome must satisfy its typed contract: {error}",
                    capability_id.as_ref()
                )
            });
            assert_eq!(outcome.capability_id(), capability_id);
            let canonical = outcome.clone().into_wire();
            let reparsed = outcome_from_output(&capability_id, canonical.clone())
                .expect("canonical outcome must parse again");
            assert_eq!(reparsed, outcome);
            assert_eq!(reparsed.into_wire(), canonical);

            let mut unknown = sample_output(capability_id.as_ref()).0;
            unknown
                .as_object_mut()
                .expect("sample output is an object")
                .insert("unknown_field".to_owned(), json!(true));
            let error = outcome_from_output(&capability_id, StrictJsonValue(unknown))
                .expect_err("unknown outcome fields must reject");
            assert_eq!(error.code, WAVE3_INVALID_RESPONSE);
            assert!(error.message.contains("unknown field `unknown_field`"));
        }
    }

    #[test]
    fn resource_owner_and_outcome_identity_are_enforced() {
        let capability_id = CapabilityId::from("workshop.canvas.read");
        let spec = find_capability(capability_id.as_ref()).expect("known capability");
        let mut bindings = bindings_for(capability_id.as_ref(), "owner-a");
        bindings[0].owner_id = "owner-b".to_owned();
        let error = validate_resource_bindings_contract(
            &capability_id,
            "owner-a",
            spec.requirements,
            &bindings,
        )
        .expect_err("cross-owner resource binding must reject");
        assert_eq!(error.code, WAVE3_RESOURCE_OWNER_MISMATCH);

        let outcome = outcome_from_output(
            &capability_id,
            sample_output(capability_id.as_ref()),
        )
        .expect("typed Canvas outcome");
        let matching = bindings_for(capability_id.as_ref(), "owner-a");
        validate_outcome_resource_binding(&outcome, &matching)
            .expect("matching bound Canvas identity");

        let mut mismatched = matching;
        mismatched[0].resource_id = ResourceId::from(ID_2);
        let error = validate_outcome_resource_binding(&outcome, &mismatched)
            .expect_err("owner must not return a different Canvas");
        assert_eq!(error.code, WAVE3_INVALID_RESPONSE);
        assert!(error.message.contains("does not match bound resource"));
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
            let operation = operation_from_input(
                &capability_id,
                sample_input(capability_id.as_ref()),
            )
            .expect("known active Wave 3 operation");
            let owner_id = "wave3-test-owner";
            let resource_bindings = bindings_for(capability_id.as_ref(), owner_id);
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
            let operation = operation_from_input(
                &capability_id,
                sample_input(capability_id.as_ref()),
            )
            .unwrap();
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
                    canvas_id: "0190f5fe-7c00-7a00-8000-000000000101".to_owned(),
                    node_id: "0190f5fe-7c00-7a00-8000-000000000102".to_owned(),
                },
                prompt: "Draft a short caption".to_owned(),
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
}
