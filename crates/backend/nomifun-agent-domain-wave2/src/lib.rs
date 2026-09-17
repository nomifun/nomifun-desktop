//! Bundled Wave 2 coding-extension capability registrations.
//!
//! The package inventory in this crate is deliberately limited to the
//! extension surface around Coding: workspace/filesystem/process/terminal/VCS,
//! SSH, Browser, and Computer/A11y. Dynamic MCP tools are published by their
//! owning extension package and are intentionally absent here. The exact native Coding
//! surface is owned by the Runtime contract and is not re-declared here.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityAuthoringPolicy, CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind,
    CapabilityManifest, CapabilityRef,
    CanonicalErrorCode, CanonicalSchemaRef, CancellationDescriptor,
    CorrelationId, DeclaredServiceViewDescriptor, EffectClass, ExactRoleContractRef,
    ExecutionRoleId, HostPortBindingDescriptor, HostPortId, HostPortRef, IdempotencyKey,
    InProcessEntrypointMetadata,
    LocalizedMetadata, ManagedTaskRegistrationDescriptor, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState, PluginEffectiveState,
    PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateCompareAndSwapOutcome, PluginStateEntry,
    PluginStateHandleDescriptor, PluginStateMethod, OperationId, PrincipalRef,
    ExactRoleProviderRef, ResolvedSnapshotRef, ResourceKind, RoleContractKey, RoleContractManifest,
    RoleMemberContract, RoleMemberRequirement, RoleProviderContribution,
    RoleProviderMemberContribution, RuntimeTarget, ScopeKey, StateKey,
    StrictJsonValue,
    ToolPresentationKind, TypedResourceBindings, ValidatedPluginConfig, VersionString,
    CAPABILITY_UNAVAILABLE_ON_PLATFORM, PRESET_RESOURCE_NOT_BOUND, RESOURCE_OWNER_MISMATCH,
    capability_module_surface_declarations, capability_surface_declarations, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityContextContributionFactory, CapabilityContextContributionRequest,
    CapabilityHandler, CapabilityInvocationContext, CapabilityResourceProviderFactory,
    CapabilityResourceProviderRequest, ContextContributionFactory, ContextContributionRequest,
    ContextContributionResult, HostPluginStateApi, KernelError, PluginRegistration,
    PluginStateError, PluginStateHandle, ResourceProviderResult, ResolvedRoleMemberContext,
};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const BROWSER_ROLE_CONTRACT_VERSION: &str = "2.0.0";
pub const VERSION: &str = CONTRACT_VERSION;
pub const PACKAGE_VERSION: &str = CONTRACT_VERSION;

pub const WORKSPACE_EXECUTION_PACKAGE_ID: &str = "nomifun.workspace-execution";
pub const SSH_PACKAGE_ID: &str = "nomifun.ssh";
pub const BROWSER_PACKAGE_ID: &str = "nomifun.browser";
pub const COMPUTER_A11Y_PACKAGE_ID: &str = "nomifun.computer-a11y";

pub const WORKSPACE_EXECUTION_MOUNT_ID: &str = "domain-workspace-execution";
pub const SSH_MOUNT_ID: &str = "domain-ssh";
pub const BROWSER_MOUNT_ID: &str = "domain-browser";
pub const COMPUTER_A11Y_MOUNT_ID: &str = "domain-computer-a11y";
pub const BROWSER_EXECUTION_ROLE_ID: &str = "system.browser_use";
pub const COMPUTER_EXECUTION_ROLE_ID: &str = "system.computer_use";

pub const PACKAGE_IDS: [&str; 4] = [
    WORKSPACE_EXECUTION_PACKAGE_ID,
    SSH_PACKAGE_ID,
    BROWSER_PACKAGE_ID,
    COMPUTER_A11Y_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 4] = PACKAGE_IDS;

pub const WORKSPACE_FILES_MODULE_ID: &str = "workspace.files";
pub const WORKSPACE_VCS_MODULE_ID: &str = "workspace.vcs";
pub const WORKSPACE_PROCESS_MODULE_ID: &str = "workspace.process";
pub const WORKSPACE_ARTIFACTS_MODULE_ID: &str = "workspace.artifacts";
pub const SSH_MODULE_ID: &str = "ssh";
pub const BROWSER_MODULE_ID: &str = "browser";
pub const WORKSPACE_FILES_CHANGED_EVENT_SCHEMA_ID: &str = "workspace.files/changed";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFileChangeKind {
    Created,
    Modified,
    Removed,
    Renamed,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFileChangedEvent {
    pub path: String,
    pub kind: WorkspaceFileChangeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFilesChangedBatch {
    pub capability_id: String,
    pub event_schema: String,
    pub events: Vec<WorkspaceFileChangedEvent>,
    pub dropped_event_count: u64,
}

impl WorkspaceFilesChangedBatch {
    pub fn new(events: Vec<WorkspaceFileChangedEvent>, dropped_event_count: u64) -> Self {
        Self {
            capability_id: WORKSPACE_FILES_MODULE_ID.to_owned(),
            event_schema: WORKSPACE_FILES_CHANGED_EVENT_SCHEMA_ID.to_owned(),
            events,
            dropped_event_count,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.capability_id != WORKSPACE_FILES_MODULE_ID
            || self.event_schema != WORKSPACE_FILES_CHANGED_EVENT_SCHEMA_ID
            || self.events.len() > 256
            || self.events.iter().any(|event| {
                event.path.is_empty()
                    || event.path.len() > 4096
                    || event.path.starts_with('/')
                    || event.path.contains(['\0', '\\'])
                    || event.path.split('/').any(|part| part.is_empty() || part == "..")
            })
        {
            return Err("workspace.files changed batch violates its canonical contract".into());
        }
        Ok(())
    }
}

pub const WORKSPACE_FILES_ACTION_IDS: &[&str] = &[
    "workspace.files/read",
    "workspace.files/search",
    "workspace.files/write",
    "workspace.files/patch",
    "workspace.files/delete",
];
pub const WORKSPACE_VCS_ACTION_IDS: &[&str] = &[
    "workspace.vcs/status",
    "workspace.vcs/diff",
    "workspace.vcs/stage",
    "workspace.vcs/commit",
    "workspace.vcs/push",
];
pub const WORKSPACE_PROCESS_ACTION_IDS: &[&str] = &[
    "workspace.process/exec",
    "workspace.process/start",
    "workspace.process/poll",
    "workspace.process/input",
    "workspace.process/close_stdin",
    "workspace.process/resize",
    "workspace.process/cancel",
];
pub const WORKSPACE_ARTIFACTS_ACTION_IDS: &[&str] = &[
    "workspace.artifacts/read",
    "workspace.artifacts/publish",
];

pub const WORKSPACE_EXECUTION_CAPABILITY_IDS: &[&str] = &[
    WORKSPACE_FILES_MODULE_ID,
    WORKSPACE_VCS_MODULE_ID,
    WORKSPACE_PROCESS_MODULE_ID,
    WORKSPACE_ARTIFACTS_MODULE_ID,
];

pub const SSH_ACTION_IDS: &[&str] = &[
    "ssh/fs.read",
    "ssh/fs.write",
    "ssh/exec",
    "ssh/sudo",
];
pub const SSH_CAPABILITY_IDS: &[&str] = &[SSH_MODULE_ID];

pub const BROWSER_ACTION_IDS: &[&str] = &[
    "browser/observe",
    "browser/navigate",
    "browser/act",
    "browser/render_content",
    "browser/download",
    "browser/upload",
    "browser/evaluate",
];
pub const BROWSER_CAPABILITY_IDS: &[&str] = &[BROWSER_MODULE_ID];

pub const COMPUTER_A11Y_CAPABILITY_IDS: &[&str] = &[
    "computer.observe",
    "computer.input",
    "computer.launch",
    "a11y.observe",
];

pub const ALL_CAPABILITY_IDS: [&str; 10] = [
    WORKSPACE_FILES_MODULE_ID,
    WORKSPACE_VCS_MODULE_ID,
    WORKSPACE_PROCESS_MODULE_ID,
    WORKSPACE_ARTIFACTS_MODULE_ID,
    SSH_MODULE_ID,
    BROWSER_MODULE_ID,
    "computer.observe",
    "computer.input",
    "computer.launch",
    "a11y.observe",
];
pub const TARGET_CAPABILITY_IDS: [&str; 10] = ALL_CAPABILITY_IDS;

pub const TARGET_CAPABILITY_FAMILIES: [&str; 10] = [
    "browser",
    "computer",
    "filesystem",
    "process",
    "remote-execution",
    "review-ci",
    "ssh",
    "terminal",
    "vcs",
    "workspace",
];

/// Browser is release-defined on all four desktop host targets.  Headless
/// hosts are intentionally absent from this set.
pub const BROWSER_DESKTOP_HOST_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
];

/// The first release does not claim a full Computer surface on Linux Desktop.
/// Linux Desktop and all headless hosts therefore fail closed with the
/// canonical platform-unavailable error.
pub const COMPUTER_DESKTOP_HOST_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
];

pub const DESKTOP_HOST_SURFACES: &[&str] = &["desktop"];
pub const AGENT_SURFACES: &[&str] = &["desktop", "headless"];
pub const BROWSER_COMPUTER_SURFACES: &[&str] = &["desktop"];

const WORKSPACE_RESOURCE: &[&str] = &["workspace"];
const PROCESS_RESOURCE: &[&str] = &["process_session"];
const SSH_RESOURCE: &[&str] = &["ssh_host"];
const BROWSER_RESOURCE: &[&str] = &["browser"];
const COMPUTER_RESOURCE: &[&str] = &["computer"];

const PLUGIN_CANCEL_PORT: &str = "host.plugin.cancel";
const PLUGIN_TASKS_PORT: &str = "host.plugin.tasks";

/// The single host port used by action-bearing Wave 2 capabilities.
///
/// The domain crate owns capability metadata and invocation validation.  The
/// host owns filesystem, process, SSH, Browser, and Computer facts and
/// must provide the real action result through this port.
pub const WAVE2_CAPABILITY_HOST_PORT_ID: &str = "host.wave2.capability.invoke";

/// Canonical error codes used by the typed Wave 2 host boundary.
///
/// `CAPABILITY_UNAVAILABLE` is intentionally distinct from
/// `CAPABILITY_UNAVAILABLE_ON_PLATFORM`: the former means that the owning
/// application adapter is not currently mounted, while the latter means that
/// the capability is not part of the host's declared platform surface.
pub const CAPABILITY_UNAVAILABLE: &str = "CAPABILITY_UNAVAILABLE";
pub const INVALID_PAYLOAD: &str = "INVALID_PAYLOAD";
pub const RESOURCE_NOT_FOUND: &str = "RESOURCE_NOT_FOUND";

/// The narrow state surface exposed to a Wave 2 owner.
///
/// The owner can inspect its Kernel-authorized namespace and perform an
/// atomic compare-and-swap transition, but it cannot receive a pool or a
/// service locator. Read/modify/write state transitions must use CAS.
#[derive(Clone)]
pub struct Wave2StateHandle(PluginStateHandle);

impl fmt::Debug for Wave2StateHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Wave2StateHandle")
            .field("descriptor", self.descriptor())
            .finish()
    }
}

impl PartialEq for Wave2StateHandle {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor() == other.descriptor()
    }
}

impl Eq for Wave2StateHandle {}

impl Wave2StateHandle {
    fn new(handle: PluginStateHandle) -> Self {
        Self(handle)
    }

    pub fn descriptor(&self) -> &PluginStateHandleDescriptor {
        self.0.descriptor()
    }

    pub async fn get(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<Option<PluginStateEntry>, PluginStateError> {
        self.0.get(scope_key, state_key).await
    }

    pub async fn compare_and_swap(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        expected_revision: u64,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        self.0
            .compare_and_swap(
                scope_key,
                state_key,
                expected_revision,
                state_format_version,
                value,
            )
            .await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave2HostContext {
    /// The authenticated principal used for owner checks; adapters must not
    /// infer identity from input payloads or resource IDs.
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    /// Canonical parent Turn whose durable Agent Effect ledger owns this
    /// invocation. Hosts must not derive this identity from the operation or
    /// from model-visible input.
    pub turn_id: OperationId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub role_provider: Option<ExactRoleProviderRef>,
    /// The frozen, authorization-bearing host bindings selected for this
    /// invocation. The application resolves these bindings; the adapter
    /// receives them without any pool or service-bag access.
    /// The state handle is similarly scoped to the mounted package and is
    /// the only state surface exposed to the adapter.
    pub state: Wave2StateHandle,
    pub resource_bindings: TypedResourceBindings,
}

/// A family-typed action operation for the host adapter.
///
/// Capability and action identity remain in [`Wave2HostContext`].  The
/// variant prevents a host adapter from treating every Wave 2 action as an
/// untyped generic success path while leaving capability-specific JSON
/// decoding to the owning host service.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave2CapabilityOperation {
    WorkspaceExecution { input: StrictJsonValue },
    Ssh { input: StrictJsonValue },
    Browser { input: StrictJsonValue },
    ComputerA11y { input: StrictJsonValue },
}

/// Exact capability-to-operation mapping exposed to composable host
/// adapters.
///
/// [`Wave2CapabilityOperation`] is the family envelope consumed by the
/// application host. This enum is the exact contract for independently owned
/// owners: each action has its own variant, so a dispatch implementation cannot
/// accidentally handle `workspace.files/delete` as `workspace.files/read` or silently treat an
/// unsupported action as a successful generic operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave2TypedCapabilityOperation {
    WorkspaceFileRead { input: StrictJsonValue },
    WorkspaceFileSearch { input: StrictJsonValue },
    WorkspaceFileWrite { input: StrictJsonValue },
    WorkspaceFilePatch { input: StrictJsonValue },
    WorkspaceFileDelete { input: StrictJsonValue },
    WorkspaceVcsStatus { input: StrictJsonValue },
    WorkspaceVcsDiff { input: StrictJsonValue },
    WorkspaceVcsStage { input: StrictJsonValue },
    WorkspaceVcsCommit { input: StrictJsonValue },
    WorkspaceVcsPush { input: StrictJsonValue },
    WorkspaceProcessExec { input: StrictJsonValue },
    WorkspaceProcessStart { input: StrictJsonValue },
    WorkspaceProcessPoll { input: StrictJsonValue },
    WorkspaceProcessInput { input: StrictJsonValue },
    WorkspaceProcessCloseStdin { input: StrictJsonValue },
    WorkspaceProcessResize { input: StrictJsonValue },
    WorkspaceProcessCancel { input: StrictJsonValue },
    WorkspaceArtifactRead { input: StrictJsonValue },
    WorkspaceArtifactPublish { input: StrictJsonValue },
    SshFsRead { input: StrictJsonValue },
    SshFsWrite { input: StrictJsonValue },
    SshExec { input: StrictJsonValue },
    SshSudo { input: StrictJsonValue },
    BrowserObserve { input: StrictJsonValue },
    BrowserNavigate { input: StrictJsonValue },
    BrowserAct { input: StrictJsonValue },
    BrowserRenderContent { input: StrictJsonValue },
    BrowserDownload { input: StrictJsonValue },
    BrowserUpload { input: StrictJsonValue },
    BrowserEvaluate { input: StrictJsonValue },
    ComputerInput { input: StrictJsonValue },
    ComputerLaunch { input: StrictJsonValue },
}

impl Wave2TypedCapabilityOperation {
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::WorkspaceFileRead { .. }
            | Self::WorkspaceFileSearch { .. }
            | Self::WorkspaceFileWrite { .. }
            | Self::WorkspaceFilePatch { .. }
            | Self::WorkspaceFileDelete { .. } => WORKSPACE_FILES_MODULE_ID,
            Self::WorkspaceVcsStatus { .. }
            | Self::WorkspaceVcsDiff { .. }
            | Self::WorkspaceVcsStage { .. }
            | Self::WorkspaceVcsCommit { .. }
            | Self::WorkspaceVcsPush { .. } => WORKSPACE_VCS_MODULE_ID,
            Self::WorkspaceProcessExec { .. }
            | Self::WorkspaceProcessStart { .. }
            | Self::WorkspaceProcessPoll { .. }
            | Self::WorkspaceProcessInput { .. }
            | Self::WorkspaceProcessCloseStdin { .. }
            | Self::WorkspaceProcessResize { .. }
            | Self::WorkspaceProcessCancel { .. } => WORKSPACE_PROCESS_MODULE_ID,
            Self::WorkspaceArtifactRead { .. }
            | Self::WorkspaceArtifactPublish { .. } => WORKSPACE_ARTIFACTS_MODULE_ID,
            Self::SshFsRead { .. }
            | Self::SshFsWrite { .. }
            | Self::SshExec { .. }
            | Self::SshSudo { .. } => SSH_MODULE_ID,
            Self::BrowserObserve { .. }
            | Self::BrowserNavigate { .. }
            | Self::BrowserAct { .. }
            | Self::BrowserRenderContent { .. }
            | Self::BrowserDownload { .. }
            | Self::BrowserUpload { .. }
            | Self::BrowserEvaluate { .. } => BROWSER_MODULE_ID,
            Self::ComputerInput { .. } => "computer.input",
            Self::ComputerLaunch { .. } => "computer.launch",
        }
    }

    pub fn action_id(&self) -> &'static str {
        match self {
            Self::WorkspaceFileRead { .. } => "workspace.files/read",
            Self::WorkspaceFileSearch { .. } => "workspace.files/search",
            Self::WorkspaceFileWrite { .. } => "workspace.files/write",
            Self::WorkspaceFilePatch { .. } => "workspace.files/patch",
            Self::WorkspaceFileDelete { .. } => "workspace.files/delete",
            Self::WorkspaceVcsStatus { .. } => "workspace.vcs/status",
            Self::WorkspaceVcsDiff { .. } => "workspace.vcs/diff",
            Self::WorkspaceVcsStage { .. } => "workspace.vcs/stage",
            Self::WorkspaceVcsCommit { .. } => "workspace.vcs/commit",
            Self::WorkspaceVcsPush { .. } => "workspace.vcs/push",
            Self::WorkspaceProcessExec { .. } => "workspace.process/exec",
            Self::WorkspaceProcessStart { .. } => "workspace.process/start",
            Self::WorkspaceProcessPoll { .. } => "workspace.process/poll",
            Self::WorkspaceProcessInput { .. } => "workspace.process/input",
            Self::WorkspaceProcessCloseStdin { .. } => "workspace.process/close_stdin",
            Self::WorkspaceProcessResize { .. } => "workspace.process/resize",
            Self::WorkspaceProcessCancel { .. } => "workspace.process/cancel",
            Self::WorkspaceArtifactRead { .. } => "workspace.artifacts/read",
            Self::WorkspaceArtifactPublish { .. } => "workspace.artifacts/publish",
            Self::SshFsRead { .. } => "ssh/fs.read",
            Self::SshFsWrite { .. } => "ssh/fs.write",
            Self::SshExec { .. } => "ssh/exec",
            Self::SshSudo { .. } => "ssh/sudo",
            Self::BrowserObserve { .. } => "browser/observe",
            Self::BrowserNavigate { .. } => "browser/navigate",
            Self::BrowserAct { .. } => "browser/act",
            Self::BrowserRenderContent { .. } => "browser/render_content",
            Self::BrowserDownload { .. } => "browser/download",
            Self::BrowserUpload { .. } => "browser/upload",
            Self::BrowserEvaluate { .. } => "browser/evaluate",
            Self::ComputerInput { .. } => "computer.input.invoke",
            Self::ComputerLaunch { .. } => "computer.launch.invoke",
        }
    }

    fn family(&self) -> Wave2CapabilityOperation {
        match self {
            Self::WorkspaceFileRead { input }
            | Self::WorkspaceFileSearch { input }
            | Self::WorkspaceFileWrite { input }
            | Self::WorkspaceFilePatch { input }
            | Self::WorkspaceFileDelete { input }
            | Self::WorkspaceVcsStatus { input }
            | Self::WorkspaceVcsDiff { input }
            | Self::WorkspaceVcsStage { input }
            | Self::WorkspaceVcsCommit { input }
            | Self::WorkspaceVcsPush { input }
            | Self::WorkspaceProcessExec { input }
            | Self::WorkspaceProcessStart { input }
            | Self::WorkspaceProcessPoll { input }
            | Self::WorkspaceProcessInput { input }
            | Self::WorkspaceProcessCloseStdin { input }
            | Self::WorkspaceProcessResize { input }
            | Self::WorkspaceProcessCancel { input }
            | Self::WorkspaceArtifactRead { input }
            | Self::WorkspaceArtifactPublish { input } => {
                Wave2CapabilityOperation::WorkspaceExecution { input: input.clone() }
            }
            Self::SshFsRead { input }
            | Self::SshFsWrite { input }
            | Self::SshExec { input }
            | Self::SshSudo { input } => Wave2CapabilityOperation::Ssh { input: input.clone() },
            Self::BrowserObserve { input }
            | Self::BrowserNavigate { input }
            | Self::BrowserAct { input }
            | Self::BrowserRenderContent { input }
            | Self::BrowserDownload { input }
            | Self::BrowserUpload { input }
            | Self::BrowserEvaluate { input } => {
                Wave2CapabilityOperation::Browser { input: input.clone() }
            }
            Self::ComputerInput { input } | Self::ComputerLaunch { input } => {
                Wave2CapabilityOperation::ComputerA11y { input: input.clone() }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave2HostRequest {
    pub context: Wave2HostContext,
    pub operation: Wave2CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave2TypedHostRequest {
    pub context: Wave2HostContext,
    pub operation: Wave2TypedCapabilityOperation,
}

impl Wave2HostRequest {
    /// Validate the family envelope and project it to the exact operation
    /// contract used by an owner-backed adapter.
    pub fn into_typed(self) -> Result<Wave2TypedHostRequest, Wave2HostPortError> {
        if !declares_action(&self.context.capability_id, &self.context.action_id) {
            return Err(Wave2HostPortError::new(
                "ACTION_NOT_DECLARED",
                format!(
                    "{} does not declare action {}",
                    self.context.capability_id.as_ref(),
                    self.context.action_id.as_ref()
                ),
            ));
        }
        let input = match &self.operation {
            Wave2CapabilityOperation::WorkspaceExecution { input }
            | Wave2CapabilityOperation::Ssh { input }
            | Wave2CapabilityOperation::Browser { input }
            | Wave2CapabilityOperation::ComputerA11y { input } => input.clone(),
        };
        let typed = typed_operation_for(
            &self.context.capability_id,
            &self.context.action_id,
            input,
        )
        .map_err(|error| Wave2HostPortError::new("ACTION_NOT_DECLARED", error.to_string()))?;
        if typed.family() != self.operation {
            return Err(Wave2HostPortError::new(
                "ACTION_OPERATION_MISMATCH",
                format!(
                    "{} was paired with the wrong typed host operation family",
                    self.context.capability_id.as_ref()
                ),
            ));
        }
        validate_action_resource_bindings(
            &self.context.capability_id,
            &self.context.action_id,
            &self.context.principal,
            &self.context.resource_bindings,
        )
        .map_err(|error| {
            Wave2HostPortError::new(error.canonical_code().as_ref().to_owned(), error.to_string())
        })?;
        Ok(Wave2TypedHostRequest { context: self.context, operation: typed })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave2HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave2HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(CAPABILITY_UNAVAILABLE, message)
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self::new(INVALID_PAYLOAD, message)
    }

    pub fn resource_not_bound(message: impl Into<String>) -> Self {
        Self::new(PRESET_RESOURCE_NOT_BOUND, message)
    }

    pub fn owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(RESOURCE_OWNER_MISMATCH, message)
    }

    pub fn platform_unavailable(message: impl Into<String>) -> Self {
        Self::new(CAPABILITY_UNAVAILABLE_ON_PLATFORM, message)
    }

    pub fn canonical_code(&self) -> CanonicalErrorCode {
        CanonicalErrorCode::from(self.code.clone())
    }
}

impl fmt::Display for Wave2HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave2HostPortError {}

/// Host-owned implementation boundary for action-bearing Wave 2 capabilities.
pub trait Wave2HostPort: Send + Sync {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>;
}

/// Trusted facts projected to non-action Role members.
#[derive(Clone)]
pub struct Wave2RoleMemberContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub role_provider: ExactRoleProviderRef,
    pub state_scope_key: ScopeKey,
    pub state: Wave2StateHandle,
    pub resource_bindings: TypedResourceBindings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave2ContextCapabilityOperation {
    ComputerObserve,
    A11yObserve,
}

impl Wave2ContextCapabilityOperation {
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::ComputerObserve => "computer.observe",
            Self::A11yObserve => "a11y.observe",
        }
    }
}


#[derive(Clone)]
pub struct Wave2ContextHostRequest {
    pub context: Wave2RoleMemberContext,
    pub operation: Wave2ContextCapabilityOperation,
    pub schema_ref: CanonicalSchemaRef,
}


pub trait Wave2ContextHostPort: Send + Sync {
    fn contribute<'a>(
        &'a self,
        request: Wave2ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ContextContributionResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >;
}


/// An exact-operation adapter used by [`Wave2HostPortDispatcher`].
pub trait Wave2TypedOperationAdapter: Send + Sync {
    fn supports(&self, operation: &Wave2TypedCapabilityOperation) -> bool;

    fn invoke<'a>(
        &'a self,
        request: Wave2TypedHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>;
}

/// Compose independently owned capability owners behind one Wave 2 host port.
///
/// Dispatch is first-match and fail-closed. An adapter is selected only by its
/// exact typed operation. The dispatcher never retries another owner after an
/// adapter has accepted an operation, so an owner remains authoritative for
/// its side effects.
pub struct Wave2HostPortDispatcher {
    adapters: Vec<Arc<dyn Wave2TypedOperationAdapter>>,
}

impl Wave2HostPortDispatcher {
    pub fn new(adapters: Vec<Arc<dyn Wave2TypedOperationAdapter>>) -> Self {
        Self { adapters }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn push(&mut self, adapter: Arc<dyn Wave2TypedOperationAdapter>) {
        self.adapters.push(adapter);
    }
}

impl Wave2HostPort for Wave2HostPortDispatcher {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>> {
        let typed = match request.into_typed() {
            Ok(typed) => typed,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let Some(adapter) = self
            .adapters
            .iter()
            .find(|adapter| adapter.supports(&typed.operation))
        else {
            let capability_id = typed.context.capability_id.clone();
            return Box::pin(async move {
                Err(Wave2HostPortError::unavailable(format!(
                    "no canonical application owner is wired for {}",
                    capability_id.as_ref()
                )))
            });
        };
        adapter.invoke(typed)
    }
}

struct ClosureWave2TypedOperationAdapter<S, F> {
    supports: S,
    dispatch: F,
}

impl<S, F> ClosureWave2TypedOperationAdapter<S, F> {
    fn new(supports: S, dispatch: F) -> Self {
        Self { supports, dispatch }
    }
}

impl<S, F, Fut> Wave2TypedOperationAdapter for ClosureWave2TypedOperationAdapter<S, F>
where
    S: Fn(&Wave2TypedCapabilityOperation) -> bool + Send + Sync + 'static,
    F: Fn(Wave2TypedHostRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'static,
{
    fn supports(&self, operation: &Wave2TypedCapabilityOperation) -> bool {
        (self.supports)(operation)
    }

    fn invoke<'a>(
        &'a self,
        request: Wave2TypedHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>> {
        let dispatch = &self.dispatch;
        Box::pin(async move { dispatch(request).await })
    }
}

/// Build one exact-operation adapter from closures for central app
/// composition without exposing this crate's private handler types.
pub fn typed_operation_adapter<S, F, Fut>(
    supports: S,
    dispatch: F,
) -> Arc<dyn Wave2TypedOperationAdapter>
where
    S: Fn(&Wave2TypedCapabilityOperation) -> bool + Send + Sync + 'static,
    F: Fn(Wave2TypedHostRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'static,
{
    Arc::new(ClosureWave2TypedOperationAdapter::new(supports, dispatch))
}

struct UnconfiguredWave2HostPort;

impl Wave2HostPort for UnconfiguredWave2HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            Err(Wave2HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

/// Return the default adapter used by metadata-only compositions.
///
/// It deliberately fails closed; it never fabricates an action result.
pub fn unconfigured_host_port() -> Arc<dyn Wave2HostPort> {
    Arc::new(UnconfiguredWave2HostPort)
}

struct UnconfiguredWave2ContextHostPort;

impl Wave2ContextHostPort for UnconfiguredWave2ContextHostPort {
    fn contribute<'a>(
        &'a self,
        request: Wave2ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ContextContributionResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >
    {
        Box::pin(async move {
            Err(Wave2HostPortError::unavailable(format!(
                "no production context owner is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}


pub fn unconfigured_context_host_port() -> Arc<dyn Wave2ContextHostPort> {
    Arc::new(UnconfiguredWave2ContextHostPort)
}


#[derive(Clone)]
pub struct Wave2RoleHostPorts {
    pub actions: Arc<dyn Wave2HostPort>,
    pub browser_actions: Arc<dyn Wave2HostPort>,
    pub computer_actions: Arc<dyn Wave2HostPort>,
    pub computer_contexts: Arc<dyn Wave2ContextHostPort>,
}

impl Wave2RoleHostPorts {
    pub fn with_actions(actions: Arc<dyn Wave2HostPort>) -> Self {
        Self {
            browser_actions: Arc::clone(&actions),
            computer_actions: Arc::clone(&actions),
            actions,
            computer_contexts: unconfigured_context_host_port(),
        }
    }

    fn action_port(&self, package_id: &str) -> Arc<dyn Wave2HostPort> {
        match package_id {
            BROWSER_PACKAGE_ID => Arc::clone(&self.browser_actions),
            COMPUTER_A11Y_PACKAGE_ID => Arc::clone(&self.computer_actions),
            _ => Arc::clone(&self.actions),
        }
    }

    fn context_port(
        &self,
        role_id: &ExecutionRoleId,
    ) -> Arc<dyn Wave2ContextHostPort> {
        match role_id.as_ref() {
            COMPUTER_EXECUTION_ROLE_ID => Arc::clone(&self.computer_contexts),
            _ => unconfigured_context_host_port(),
        }
    }

}

struct Wave2ContextFactory {
    role_id: ExecutionRoleId,
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave2ContextHostPort>,
}

struct Wave2UnavailableCapabilityContextFactory {
    capability_id: CapabilityId,
}

#[async_trait::async_trait]
impl CapabilityContextContributionFactory for Wave2UnavailableCapabilityContextFactory {
    async fn contribute(
        &self,
        _request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        Err(KernelError::CapabilityExecution {
            reason: format!(
                "Wave 2 Context capability {} has no configured context owner",
                self.capability_id.as_ref()
            ),
        })
    }
}

struct Wave2UnavailableCapabilityResourceFactory {
    capability_id: CapabilityId,
}

#[async_trait::async_trait]
impl CapabilityResourceProviderFactory for Wave2UnavailableCapabilityResourceFactory {
    async fn acquire(
        &self,
        _request: CapabilityResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        Err(KernelError::CapabilityExecution {
            reason: format!(
                "Wave 2 Resource capability {} has no configured resource owner",
                self.capability_id.as_ref()
            ),
        })
    }
}

#[async_trait::async_trait]
impl ContextContributionFactory for Wave2ContextFactory {
    async fn contribute(
        &self,
        request: ContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        if request.context.provider_lock.provider.role.key.role_id != self.role_id
            || request.context.member_id != self.capability_id
        {
            return Err(KernelError::RoleProviderMemberUnavailable {
                role_id: self.role_id.clone(),
                capability_id: self.capability_id.clone(),
            });
        }
        let operation = match self.capability_id.as_ref() {
            "computer.observe" => Wave2ContextCapabilityOperation::ComputerObserve,
            "a11y.observe" => Wave2ContextCapabilityOperation::A11yObserve,
            _ => {
                return Err(KernelError::RoleProviderMemberUnavailable {
                    role_id: self.role_id.clone(),
                    capability_id: self.capability_id.clone(),
                });
            }
        };
        self.host_port
            .contribute(Wave2ContextHostRequest {
                context: role_member_context(request.context)?,
                operation,
                schema_ref: request.schema_ref,
            })
            .await
            .map_err(wave2_host_error_to_kernel)
    }
}


fn role_member_context(
    context: ResolvedRoleMemberContext,
) -> Result<Wave2RoleMemberContext, KernelError> {
    let agent_session_id = context
        .agent_session_id
        .ok_or_else(|| KernelError::CapabilityExecution {
            reason: "role member context requires an AgentSession".to_owned(),
        })?;
    let resolved_snapshot_ref = context
        .resolved_snapshot_ref
        .ok_or_else(|| KernelError::CapabilityExecution {
            reason: "role member context requires a frozen Snapshot".to_owned(),
        })?;
    Ok(Wave2RoleMemberContext {
        principal: context.principal,
        agent_session_id,
        operation_id: context.operation_id,
        correlation_id: context.correlation_id,
        resolved_snapshot_ref,
        registry_generation: context.registry_generation,
        capability_id: context.member_id,
        role_provider: context.provider_lock.provider,
        state_scope_key: context.state_scope_key,
        state: Wave2StateHandle::new(context.mount.state),
        resource_bindings: context.resource_bindings,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlatformScope {
    Any,
    BrowserDesktop,
    ComputerDesktop,
}

#[derive(Clone, Copy)]
struct ActionDefinition {
    id: &'static str,
    effect_class: EffectClass,
    presentation: ToolPresentationKind,
}

impl ActionDefinition {
    const fn function(id: &'static str, effect_class: EffectClass) -> Self {
        Self {
            id,
            effect_class,
            presentation: ToolPresentationKind::FunctionTool,
        }
    }
}

const NO_ACTIONS: &[ActionDefinition] = &[];

#[derive(Clone, Copy)]
struct CapabilityDefinition {
    id: &'static str,
    kind: CapabilityKind,
    effect_class: Option<EffectClass>,
    module_actions: &'static [ActionDefinition],
    publishes_event: bool,
    resource_kinds: &'static [&'static str],
    platform_scope: PlatformScope,
}

impl CapabilityDefinition {
    const fn module(
        id: &'static str,
        actions: &'static [ActionDefinition],
        publishes_event: bool,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self::module_on(id, actions, publishes_event, resource_kinds, PlatformScope::Any)
    }

    const fn module_on(
        id: &'static str,
        actions: &'static [ActionDefinition],
        publishes_event: bool,
        resource_kinds: &'static [&'static str],
        platform_scope: PlatformScope,
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: None,
            module_actions: actions,
            publishes_event,
            resource_kinds,
            platform_scope,
        }
    }

    const fn context(
        id: &'static str,
        resource_kinds: &'static [&'static str],
        platform_scope: PlatformScope,
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::ContextContributor,
            effect_class: None,
            module_actions: NO_ACTIONS,
            publishes_event: false,
            resource_kinds,
            platform_scope,
        }
    }

    const fn is_tool(self) -> bool {
        self.effect_class.is_some() || !self.module_actions.is_empty()
    }
}

#[derive(Clone, Copy)]
struct PackageDefinition {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    mount_id: &'static str,
    capabilities: &'static [CapabilityDefinition],
}

const WORKSPACE_FILES_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("workspace.files/read", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.files/search", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.files/write", EffectClass::WriteDurable),
    ActionDefinition::function("workspace.files/patch", EffectClass::WriteReversible),
    ActionDefinition::function("workspace.files/delete", EffectClass::Destructive),
];

const WORKSPACE_VCS_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("workspace.vcs/status", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.vcs/diff", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.vcs/stage", EffectClass::WriteReversible),
    ActionDefinition::function("workspace.vcs/commit", EffectClass::WriteDurable),
    ActionDefinition::function("workspace.vcs/push", EffectClass::ExternalTransmit),
];

const WORKSPACE_PROCESS_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("workspace.process/exec", EffectClass::ExecuteLocal),
    ActionDefinition::function("workspace.process/start", EffectClass::ExecuteLocal),
    ActionDefinition::function("workspace.process/poll", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.process/input", EffectClass::ExecuteLocal),
    ActionDefinition::function("workspace.process/close_stdin", EffectClass::ExecuteLocal),
    ActionDefinition::function("workspace.process/resize", EffectClass::ExecuteLocal),
    ActionDefinition::function("workspace.process/cancel", EffectClass::ExecuteLocal),
];

const WORKSPACE_ARTIFACT_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("workspace.artifacts/read", EffectClass::ReadLocal),
    ActionDefinition::function("workspace.artifacts/publish", EffectClass::WriteDurable),
];

const WORKSPACE_EXECUTION_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::module(
        WORKSPACE_FILES_MODULE_ID,
        WORKSPACE_FILES_ACTIONS,
        true,
        WORKSPACE_RESOURCE,
    ),
    CapabilityDefinition::module(
        WORKSPACE_VCS_MODULE_ID,
        WORKSPACE_VCS_ACTIONS,
        false,
        WORKSPACE_RESOURCE,
    ),
    CapabilityDefinition::module(
        WORKSPACE_PROCESS_MODULE_ID,
        WORKSPACE_PROCESS_ACTIONS,
        false,
        PROCESS_RESOURCE,
    ),
    CapabilityDefinition::module(
        WORKSPACE_ARTIFACTS_MODULE_ID,
        WORKSPACE_ARTIFACT_ACTIONS,
        false,
        WORKSPACE_RESOURCE,
    ),
];

const SSH_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("ssh/fs.read", EffectClass::ReadSensitive),
    ActionDefinition::function("ssh/fs.write", EffectClass::WriteDurable),
    ActionDefinition::function("ssh/exec", EffectClass::ExecuteLocal),
    ActionDefinition::function("ssh/sudo", EffectClass::ExecuteLocal),
];

const SSH_CAPABILITIES: &[CapabilityDefinition] = &[CapabilityDefinition::module(
    SSH_MODULE_ID,
    SSH_ACTIONS,
    false,
    SSH_RESOURCE,
)];

const BROWSER_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition::function("browser/observe", EffectClass::ReadSensitive),
    ActionDefinition::function("browser/navigate", EffectClass::ExternalTransmit),
    ActionDefinition::function("browser/act", EffectClass::WriteReversible),
    ActionDefinition::function("browser/render_content", EffectClass::ReadSensitive),
    ActionDefinition::function("browser/download", EffectClass::WriteDurable),
    ActionDefinition::function("browser/upload", EffectClass::ExternalTransmit),
    ActionDefinition::function("browser/evaluate", EffectClass::ExecuteLocal),
];

const BROWSER_CAPABILITIES: &[CapabilityDefinition] = &[CapabilityDefinition::module_on(
    BROWSER_MODULE_ID,
    BROWSER_ACTIONS,
    false,
    BROWSER_RESOURCE,
    PlatformScope::BrowserDesktop,
)];

const COMPUTER_A11Y_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::context(
        "computer.observe",
        COMPUTER_RESOURCE,
        PlatformScope::ComputerDesktop,
    ),
    CapabilityDefinition::computer_tool("computer.input", EffectClass::Physical),
    CapabilityDefinition::computer_tool("computer.launch", EffectClass::ExecuteLocal),
    CapabilityDefinition::context(
        "a11y.observe",
        COMPUTER_RESOURCE,
        PlatformScope::ComputerDesktop,
    ),
];

impl CapabilityDefinition {
    const fn computer_tool(id: &'static str, effect_class: EffectClass) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: Some(effect_class),
            module_actions: NO_ACTIONS,
            publishes_event: false,
            resource_kinds: COMPUTER_RESOURCE,
            platform_scope: PlatformScope::ComputerDesktop,
        }
    }
}

const PACKAGE_DEFINITIONS: &[PackageDefinition] = &[
    PackageDefinition {
        id: WORKSPACE_EXECUTION_PACKAGE_ID,
        display_name: "Workspace & Execution",
        description: "Filesystem, workspace, process, terminal, and VCS capabilities.",
        mount_id: WORKSPACE_EXECUTION_MOUNT_ID,
        capabilities: WORKSPACE_EXECUTION_CAPABILITIES,
    },
    PackageDefinition {
        id: SSH_PACKAGE_ID,
        display_name: "SSH",
        description: "Typed remote filesystem and process capabilities over SSH.",
        mount_id: SSH_MOUNT_ID,
        capabilities: SSH_CAPABILITIES,
    },
    PackageDefinition {
        id: BROWSER_PACKAGE_ID,
        display_name: "Browser",
        description: "Browser identity, observation, navigation, and interaction.",
        mount_id: BROWSER_MOUNT_ID,
        capabilities: BROWSER_CAPABILITIES,
    },
    PackageDefinition {
        id: COMPUTER_A11Y_PACKAGE_ID,
        display_name: "Computer & Accessibility",
        description: "Desktop Computer and accessibility observation capabilities.",
        mount_id: COMPUTER_A11Y_MOUNT_ID,
        capabilities: COMPUTER_A11Y_CAPABILITIES,
    },
];

/// Build the complete Wave 2 bundled registration inventory with a
/// fail-closed host adapter.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Build the complete Wave 2 bundled registration inventory with an
/// application-owned action host port.
pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave2HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    registrations_with_role_host_ports(Wave2RoleHostPorts::with_actions(action_host_port))
}

/// Build the complete Wave 2 registration inventory with independently typed
/// action, context, and resource host ports.
pub fn registrations_with_role_host_ports(
    role_host_ports: Wave2RoleHostPorts,
) -> Result<Vec<PluginRegistration>, String> {
    let mut registrations = Vec::with_capacity(PACKAGE_DEFINITIONS.len());
    let mut packages = BTreeSet::new();
    let mut mounts = BTreeSet::new();
    let mut capabilities = BTreeSet::new();

    for package in PACKAGE_DEFINITIONS {
        if !packages.insert(package.id) {
            return Err(format!("duplicate Wave 2 package {}", package.id));
        }
        if !mounts.insert(package.mount_id) {
            return Err(format!("duplicate Wave 2 mount {}", package.mount_id));
        }
        let registration = build_registration(package, role_host_ports.clone())?;
        for capability in &registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
        {
            if !capabilities.insert(capability.id.clone()) {
                return Err(format!(
                    "duplicate Wave 2 capability {}",
                    capability.id.as_ref()
                ));
            }
        }
        registrations.push(registration);
    }

    if capabilities.len() != ALL_CAPABILITY_IDS.len() {
        return Err(format!(
            "Wave 2 capability inventory has {} entries; expected {}",
            capabilities.len(),
            ALL_CAPABILITY_IDS.len()
        ));
    }

    Ok(registrations)
}

fn build_registration(
    package: &PackageDefinition,
    role_host_ports: Wave2RoleHostPorts,
) -> Result<PluginRegistration, String> {
    let action_host_port = role_host_ports.action_port(package.id);
    let package_ref = PackageRef {
        id: PackageId::from(package.id),
        version: VersionString::from(CONTRACT_VERSION),
    };
    let mut capability_manifests = Vec::with_capacity(package.capabilities.len());
    let mut handlers = Vec::new();
    let mut role_handlers = Vec::new();

    for definition in package.capabilities {
        let capability = build_capability(&package_ref, *definition)?;
        if definition.is_tool() {
            let capability_id = CapabilityId::from(definition.id);
            if let Some(role_id) = role_id_for_capability(definition.id) {
                role_handlers.push((role_id, capability_id.clone()));
            } else {
                handlers.push(capability_id.clone());
            }
        }
        capability_manifests.push(capability);
    }

    let role_contracts = role_contracts_for_package(package, &capability_manifests)?;
    let role_providers = role_providers_for_package(package, &role_contracts)?;
    let config_schema = object_schema();
    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package_ref.id.clone(),
        package_version: package_ref.version.clone(),
        display: localized(package.display_name, package.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", package.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        }
        .into(),
        contributions: PackageContributions {
            capabilities: capability_manifests,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: role_contracts.clone(),
            role_providers,
        },
    };

    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: package.id.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package_ref.clone(),
        mount_id: PluginMountId::from(package.mount_id),
    };
    let cancellation_port = host_port(PLUGIN_CANCEL_PORT);
    let tasks_port = host_port(PLUGIN_TASKS_PORT);
    let action_port = host_port(WAVE2_CAPABILITY_HOST_PORT_ID);
    let has_action_handler = !handlers.is_empty() || !role_handlers.is_empty();
    let has_role_handler = !role_handlers.is_empty()
        || package.capabilities.iter().any(|definition| {
            role_id_for_capability(definition.id).is_some()
                && matches!(
                    definition.kind,
                    CapabilityKind::ContextContributor | CapabilityKind::ResourceProvider
                )
        });
    let mut declared_host_ports =
        BTreeSet::from([cancellation_port.id.clone(), tasks_port.id.clone()]);
    if has_action_handler {
        declared_host_ports.insert(action_port.id.clone());
    }
    let host_port_bindings = if has_action_handler {
        vec![host_port_binding()?]
    } else {
        Vec::new()
    };
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest)
            .map_err(|error| format!("build {} manifest: {error}", package.id))?,
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
            ])
            .into_iter()
            .chain(
                has_role_handler
                    .then_some(PluginRegistrarOperation::ContributeRoleProvider),
            )
            .collect(),
            declared_capability_ids: package
                .capabilities
                .iter()
                .map(|definition| CapabilityId::from(definition.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: role_contracts
                .iter()
                .map(|contract| contract.key.role_id.clone())
                .collect(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context: PluginContextDescriptor {
            identity: identity.clone(),
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema)
                    .map_err(|error| format!("digest {} config: {error}", package.id))?,
                config_revision: 1,
                value: empty_object(),
            },
            state: PluginStateHandleDescriptor {
                package_id: package_ref.id.clone(),
                mount_id: identity.mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: host_port_bindings,
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", package.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: tasks_port,
                scope_key: ScopeKey::from(format!("mount:{}", package.mount_id)),
            },
        },
    };

    let mut registration = PluginRegistration::new(metadata);
    for capability_id in handlers {
        registration
            .add_capability_handler(
                capability_id.clone(),
                Arc::new(Wave2CapabilityHandler {
                    capability_id,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| format!("register {} handler: {error}", package.id))?;
    }
    for (role_id, capability_id) in role_handlers {
        registration
            .add_role_action_handler(
                role_id,
                capability_id.clone(),
                Arc::new(Wave2CapabilityHandler {
                    capability_id,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| format!("register {} role handler: {error}", package.id))?;
    }
    for definition in package.capabilities {
        if role_id_for_capability(definition.id).is_some() {
            continue;
        }
        let capability_id = CapabilityId::from(definition.id);
        match definition.kind {
            CapabilityKind::ContextContributor => registration
                .add_capability_context_factory(
                    capability_id.clone(),
                    Arc::new(Wave2UnavailableCapabilityContextFactory { capability_id }),
                )
                .map_err(|error| {
                    format!("register {} direct context factory: {error}", package.id)
                })?,
            CapabilityKind::ResourceProvider => registration
                .add_capability_resource_factory(
                    capability_id.clone(),
                    Arc::new(Wave2UnavailableCapabilityResourceFactory { capability_id }),
                )
                .map_err(|error| {
                    format!("register {} direct resource factory: {error}", package.id)
                })?,
            _ => {}
        }
    }
    for definition in package.capabilities {
        let Some(role_id) = role_id_for_capability(definition.id) else {
            continue;
        };
        let capability_id = CapabilityId::from(definition.id);
        match definition.kind {
            CapabilityKind::ContextContributor => {
                registration
                    .add_role_context_factory(
                        role_id.clone(),
                        capability_id.clone(),
                        Arc::new(Wave2ContextFactory {
                            host_port: role_host_ports.context_port(&role_id),
                            role_id,
                            capability_id,
                        }),
                    )
                    .map_err(|error| {
                        format!("register {} context factory: {error}", package.id)
                    })?;
            }
            _ => {}
        }
    }
    Ok(registration)
}

fn build_capability(
    package: &PackageRef,
    definition: CapabilityDefinition,
) -> Result<CapabilityManifest, String> {
    let actions = if !definition.module_actions.is_empty() {
        definition
            .module_actions
            .iter()
            .map(|action| {
                Ok(CapabilityActionDescriptor {
                    action_id: ActionId::from(action.id),
                    input_schema: schema_ref(action.id, "input")?,
                    output_schema: schema_ref(action.id, "output")?,
                    effect_class: action.effect_class,
                    presentation: action.presentation,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    } else if let Some(effect_class) = definition.effect_class {
        vec![CapabilityActionDescriptor {
            action_id: ActionId::from(format!("{}.invoke", definition.id)),
            input_schema: schema_ref(definition.id, "input")?,
            output_schema: schema_ref(definition.id, "output")?,
            effect_class,
            presentation: ToolPresentationKind::FunctionTool,
        }]
    } else {
        Vec::new()
    };
    let context_schema_refs = (definition.kind == CapabilityKind::ContextContributor)
        .then(|| schema_ref(definition.id, "context"))
        .transpose()?
        .into_iter()
        .collect();
    let event_schema_refs = definition
        .publishes_event
        .then(|| schema_ref(definition.id, "event"))
        .transpose()?
        .into_iter()
        .collect();

    let supported_surfaces = match role_id_for_capability(definition.id).as_ref().map(AsRef::as_ref)
    {
        Some(BROWSER_EXECUTION_ROLE_ID) => BROWSER_COMPUTER_SURFACES,
        Some(COMPUTER_EXECUTION_ROLE_ID) => BROWSER_COMPUTER_SURFACES,
        _ => match definition.platform_scope {
            PlatformScope::Any => AGENT_SURFACES,
            PlatformScope::BrowserDesktop | PlatformScope::ComputerDesktop => {
                BROWSER_COMPUTER_SURFACES
            }
        },
    };
    let supported_platforms = if role_id_for_capability(definition.id).is_some() {
        vec![PlatformConstraint::Any]
    } else {
        platform_constraints(definition.platform_scope)
    };

    Ok(CapabilityManifest {
        id: CapabilityId::from(definition.id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "{}:{}",
            if definition.module_actions.is_empty() { "capability" } else { "module" },
            definition.id
        )),
        version: VersionString::from(CONTRACT_VERSION),
        kind: definition.kind,
        package: package.clone(),
        display: {
            let (name, description) = capability_display(definition.id);
            localized(name, description)
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: if definition.module_actions.is_empty() {
            capability_surface_declarations(
                supported_surfaces.iter().copied(),
                supported_consumers(definition.id),
            )
        } else {
            capability_module_surface_declarations(
                supported_surfaces.iter().copied(),
                supported_consumers(definition.id),
                CapabilityAuthoringPolicy::Direct,
            )
        },
        requires_runtime_features: Vec::new(),
        supported_platforms,
        config_schema: object_schema(),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs,
            context_phase: Default::default(),
            ui_slot: None,
            event_schema_refs,
            resource_kinds: definition
                .resource_kinds
                .iter()
                .map(|resource_kind| ResourceKind::from(*resource_kind))
                .collect(),
            host_ports: definition
                .is_tool()
                .then(|| host_port(WAVE2_CAPABILITY_HOST_PORT_ID))
                .into_iter()
                .collect(),
        },
    })
}

mod process_schema;
mod workspace_schema;

fn action_input_schema(action_id: &str) -> StrictJsonValue {
    if let Some(schema) = workspace_schema::input(action_id) {
        return schema;
    }
    if let Some(schema) = process_schema::process_action_input_schema(action_id) {
        return schema;
    }
    match action_id {
        "ssh/fs.read" => StrictJsonValue(serde_json::json!({
            "oneOf": [
                {
                    "type":"object",
                    "additionalProperties":false,
                    "properties":{
                        "operation":{"const":"read"},
                        "path":{"type":"string","minLength":1,"maxLength":4096}
                    },
                    "required":["operation","path"]
                },
                {
                    "type":"object",
                    "additionalProperties":false,
                    "properties":{
                        "operation":{"const":"grep"},
                        "pattern":{"type":"string","minLength":1,"maxLength":16384},
                        "path":{"type":"string","minLength":1,"maxLength":4096}
                    },
                    "required":["operation","pattern","path"]
                },
                {
                    "type":"object",
                    "additionalProperties":false,
                    "properties":{
                        "operation":{"const":"list"},
                        "glob":{"type":"string","minLength":1,"maxLength":4096}
                    },
                    "required":["operation","glob"]
                },
                {
                    "type":"object",
                    "additionalProperties":false,
                    "properties":{
                        "operation":{"const":"stat"},
                        "path":{"type":"string","minLength":1,"maxLength":4096}
                    },
                    "required":["operation","path"]
                }
            ]
        })),
        "ssh/fs.write" => strict_object_schema(
            serde_json::json!({
                "path":{"type":"string","minLength":1,"maxLength":4096},
                "content":{"type":"string","maxLength":1048576}
            }),
            &["path", "content"],
        ),
        "ssh/exec" | "ssh/sudo" => strict_object_schema(
            serde_json::json!({
                "command":{"type":"string","minLength":1,"maxLength":65536},
                "timeout_ms":{"type":"integer","minimum":1,"maximum":600000}
            }),
            &["command"],
        ),
        "browser/observe" => strict_object_schema(
            serde_json::json!({"tab_id":{"type":"string","minLength":1,"maxLength":512}}),
            &[],
        ),
        "browser/navigate" => strict_object_schema(
            serde_json::json!({
                "url":{"type":"string","minLength":1,"maxLength":8192},
                "new_tab":{"type":"boolean"}
            }),
            &["url"],
        ),
        "browser/act" => strict_object_schema(
            serde_json::json!({
                "action":{"type":"object","minProperties":1}
            }),
            &["action"],
        ),
        "browser/render_content" => strict_object_schema(
            serde_json::json!({"url":{"type":"string","minLength":1,"maxLength":8192,"pattern":"^https?://"}}),
            &["url"],
        ),
        "browser/download" => strict_object_schema(
            serde_json::json!({"element":{"type":"object","minProperties":1}}),
            &["element"],
        ),
        "browser/upload" => strict_object_schema(
            serde_json::json!({
                "element":{"type":"object","minProperties":1},
                "files":{
                    "type":"array",
                    "items":{"type":"string","minLength":1,"maxLength":4096},
                    "minItems":1,
                    "maxItems":16
                }
            }),
            &["element", "files"],
        ),
        "browser/evaluate" => strict_object_schema(
            serde_json::json!({"request":{"type":"object","minProperties":1}}),
            &["request"],
        ),
        _ => open_object_schema(),
    }
}

fn canonical_schema(schema_owner: &str, role: &str) -> StrictJsonValue {
    if schema_owner == "browser/render_content" && role == "output" {
        return strict_object_schema(serde_json::json!({
            "final_url":{"type":"string","maxLength":8192},
            "html":{"type":"string","maxLength":262144},
            "html_truncated":{"type":"boolean"}
        }), &["final_url","html","html_truncated"]);
    }
    if schema_owner == WORKSPACE_FILES_MODULE_ID && role == "event" {
        return strict_object_schema(
            serde_json::json!({
                "capability_id":{"type":"string","const":WORKSPACE_FILES_MODULE_ID},
                "event_schema":{"type":"string","const":WORKSPACE_FILES_CHANGED_EVENT_SCHEMA_ID},
                "events":{
                    "type":"array",
                    "maxItems":256,
                    "items":{
                        "type":"object",
                        "additionalProperties":false,
                        "properties":{
                            "path":{"type":"string","minLength":1,"maxLength":4096},
                            "kind":{"type":"string","enum":["created","modified","removed","renamed","other"]}
                        },
                        "required":["path","kind"]
                    }
                },
                "dropped_event_count":{"type":"integer","minimum":0}
            }),
            &["capability_id", "event_schema", "events", "dropped_event_count"],
        );
    }
    match role {
        "input" => action_input_schema(schema_owner),
        "request" => open_object_schema(),
        "output" | "response" => output_schema(),
        _ => object_schema(),
    }
}

/// Resolve an action schema only when the capability, facet and digest match
/// the exact schema used by this Wave 2 registration.
pub fn resolve_action_schema(
    capability_id: &str,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    let definition = definition_for(capability_id)
        .filter(|definition| definition.is_tool())
        .ok_or_else(|| format!("unknown Wave 2 action capability {capability_id}"))?;
    let schema_owners = if definition.module_actions.is_empty() {
        vec![definition.id]
    } else {
        definition
            .module_actions
            .iter()
            .map(|action| action.id)
            .collect()
    };
    for schema_owner in schema_owners {
        for role in ["input", "output"] {
            let schema = canonical_schema(schema_owner, role);
            if schema_ref(schema_owner, role)?.as_ref() == reference.as_ref() {
                return Ok(schema);
            }
        }
    }
    Err(format!(
        "schema {} is not owned by Wave 2 capability {capability_id}",
        reference.as_ref()
    ))
}

/// Validate an untrusted action payload against the same canonical input
/// schema used to construct the capability manifest. Nomi's application host
/// calls this before selecting an execution owner, so an invalid payload
/// cannot reserve effect state or touch a filesystem/Git service.
pub fn validate_module_action_input(
    capability_id: &str,
    action_id: &str,
    input: &StrictJsonValue,
) -> Result<(), String> {
    let capability = CapabilityId::from(capability_id);
    let action = ActionId::from(action_id);
    if !declares_action(&capability, &action) {
        return Err(format!(
            "Wave 2 capability {capability_id} does not declare action {action_id}"
        ));
    }
    let schema_owner = if definition_for(capability_id)
        .is_some_and(|definition| definition.module_actions.is_empty())
    {
        capability_id
    } else {
        action_id
    };
    let schema = action_input_schema(schema_owner);
    let validator = jsonschema::options()
        .build(&schema.0)
        .map_err(|error| {
            format!("compile {capability_id}/{action_id} canonical input schema: {error}")
        })?;
    validator.validate(&input.0).map_err(|error| {
        format!("{capability_id}/{action_id} input does not match its canonical schema: {error}")
    })
}

/// Validate a single-action capability used by existing Browser/Computer/SSH
/// consumers. Workspace Modules must use [`validate_module_action_input`]
/// because choosing a Module never implies choosing one of its Actions.
pub fn validate_action_input(
    capability_id: &str,
    input: &StrictJsonValue,
) -> Result<(), String> {
    let actions = action_ids(capability_id);
    if actions.len() == 1 {
        let action = actions.into_iter().next().expect("one checked Action");
        return validate_module_action_input(capability_id, action.as_ref(), input);
    }
    Err(format!(
        "Wave 2 capability {capability_id} does not have one unambiguous Action"
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

pub fn supported_consumers(capability_id: &str) -> BTreeSet<CapabilityConsumer> {
    let _ = capability_id;
    BTreeSet::from([CapabilityConsumer::Agent])
}

struct Wave2CapabilityHandler {
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave2HostPort>,
}

impl CapabilityHandler for Wave2CapabilityHandler {
    fn invoke<'life0, 'async_trait>(
        &'life0 self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, KernelError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            if context.capability_id != self.capability_id
                || !declares_action(&self.capability_id, &context.action_id)
            {
                return Err(KernelError::ActionNotDeclared {
                    capability_id: context.capability_id,
                    action_id: context.action_id,
                });
            }
            if self.capability_id.as_ref().starts_with("workspace.") {
                // These actions publish strict schemas; every Kernel host must
                // enforce them before dispatch, not only the Nomi wrapper.
                validate_module_action_input(
                    self.capability_id.as_ref(),
                    context.action_id.as_ref(),
                    &input,
                )
                .map_err(|reason| {
                    KernelError::capability_execution_failed(INVALID_PAYLOAD, reason)
                })?;
            }
            if !input.0.is_object() {
                return Err(KernelError::CapabilityExecution {
                    reason: format!(
                        "{} input must be a JSON object",
                        self.capability_id.as_ref()
                    ),
                });
            }

            validate_action_resource_bindings(
                &self.capability_id,
                &context.action_id,
                &context.principal,
                &context.resource_bindings,
            )?;

            let operation = operation_for(&self.capability_id, &context.action_id, input)?;
            self.host_port
                .invoke(Wave2HostRequest {
                    context: Wave2HostContext {
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
                        role_provider: context
                            .role_provider
                            .as_ref()
                            .map(|provider| provider.provider.clone()),
                        state: Wave2StateHandle::new(context.state),
                        resource_bindings: context.resource_bindings,
                    },
                    operation,
                })
                .await
                .map_err(wave2_host_error_to_kernel)
        })
    }
}

fn wave2_host_error_to_kernel(error: Wave2HostPortError) -> KernelError {
    KernelError::capability_execution_failed(error.canonical_code(), error.message)
}

fn wave2_input_error_to_kernel(error: KernelError) -> KernelError {
    match error {
        KernelError::CapabilityExecution { reason } => {
            KernelError::capability_execution_failed(INVALID_PAYLOAD, reason)
        }
        other => other,
    }
}

fn operation_for(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    input: StrictJsonValue,
) -> Result<Wave2CapabilityOperation, KernelError> {
    Ok(typed_operation_for(capability_id, action_id, input)
        .map_err(wave2_input_error_to_kernel)?
        .family())
}

/// Map an action capability to its exact typed host operation.
///
/// This is deliberately exhaustive over the action inventory. Non-action
/// capabilities (providers, contexts, transports, and event sources) return an
/// error instead of being admitted to an action dispatcher.
pub fn typed_operation_for(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    input: StrictJsonValue,
) -> Result<Wave2TypedCapabilityOperation, KernelError> {
    let operation = match (capability_id.as_ref(), action_id.as_ref()) {
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/read") => {
            Wave2TypedCapabilityOperation::WorkspaceFileRead { input }
        }
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/search") => {
            Wave2TypedCapabilityOperation::WorkspaceFileSearch { input }
        }
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/write") => {
            Wave2TypedCapabilityOperation::WorkspaceFileWrite { input }
        }
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/patch") => {
            Wave2TypedCapabilityOperation::WorkspaceFilePatch { input }
        }
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/delete") => {
            Wave2TypedCapabilityOperation::WorkspaceFileDelete { input }
        }
        (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/status") => {
            Wave2TypedCapabilityOperation::WorkspaceVcsStatus { input }
        }
        (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/diff") => {
            Wave2TypedCapabilityOperation::WorkspaceVcsDiff { input }
        }
        (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/stage") => {
            Wave2TypedCapabilityOperation::WorkspaceVcsStage { input }
        }
        (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/commit") => {
            Wave2TypedCapabilityOperation::WorkspaceVcsCommit { input }
        }
        (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/push") => {
            Wave2TypedCapabilityOperation::WorkspaceVcsPush { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/exec") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessExec { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/start") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessStart { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/poll") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessPoll { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/input") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessInput { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/close_stdin") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessCloseStdin { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/resize") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessResize { input }
        }
        (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/cancel") => {
            Wave2TypedCapabilityOperation::WorkspaceProcessCancel { input }
        }
        (WORKSPACE_ARTIFACTS_MODULE_ID, "workspace.artifacts/read") => {
            Wave2TypedCapabilityOperation::WorkspaceArtifactRead { input }
        }
        (WORKSPACE_ARTIFACTS_MODULE_ID, "workspace.artifacts/publish") => {
            Wave2TypedCapabilityOperation::WorkspaceArtifactPublish { input }
        }
        (SSH_MODULE_ID, "ssh/fs.read") => Wave2TypedCapabilityOperation::SshFsRead { input },
        (SSH_MODULE_ID, "ssh/fs.write") => Wave2TypedCapabilityOperation::SshFsWrite { input },
        (SSH_MODULE_ID, "ssh/exec") => Wave2TypedCapabilityOperation::SshExec { input },
        (SSH_MODULE_ID, "ssh/sudo") => Wave2TypedCapabilityOperation::SshSudo { input },
        (BROWSER_MODULE_ID, "browser/observe") => Wave2TypedCapabilityOperation::BrowserObserve { input },
        (BROWSER_MODULE_ID, "browser/navigate") => Wave2TypedCapabilityOperation::BrowserNavigate { input },
        (BROWSER_MODULE_ID, "browser/act") => Wave2TypedCapabilityOperation::BrowserAct { input },
        (BROWSER_MODULE_ID, "browser/render_content") => {
            Wave2TypedCapabilityOperation::BrowserRenderContent { input }
        }
        (BROWSER_MODULE_ID, "browser/download") => Wave2TypedCapabilityOperation::BrowserDownload { input },
        (BROWSER_MODULE_ID, "browser/upload") => Wave2TypedCapabilityOperation::BrowserUpload { input },
        (BROWSER_MODULE_ID, "browser/evaluate") => Wave2TypedCapabilityOperation::BrowserEvaluate { input },
        ("computer.input", "computer.input.invoke") => Wave2TypedCapabilityOperation::ComputerInput { input },
        ("computer.launch", "computer.launch.invoke") => Wave2TypedCapabilityOperation::ComputerLaunch { input },
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} does not expose action {} through the Wave 2 host",
                    capability_id.as_ref(),
                    action_id.as_ref()
                ),
            });
        }
    };
    Ok(operation)
}

/// Return the complete canonical Wave 2 Capability ID set.
pub fn capability_ids() -> BTreeSet<CapabilityId> {
    PACKAGE_DEFINITIONS
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .map(|definition| CapabilityId::from(definition.id))
        .collect()
}

/// Return the target inventory capability set under the conventional API
/// name used by the other domain-wave crates.
pub fn target_capability_ids() -> BTreeSet<CapabilityId> {
    capability_ids()
}

/// Return the five target inventory package IDs.
pub fn package_ids() -> BTreeSet<PackageId> {
    PACKAGE_IDS
        .iter()
        .map(|package_id| PackageId::from(*package_id))
        .collect()
}

/// Return the declared typed resource kinds for a Capability.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    definition_for(capability_id).map(|definition| {
        definition
            .resource_kinds
            .iter()
            .map(|resource_kind| ResourceKind::from(*resource_kind))
            .collect()
    })
}

/// Return the primary operation that must be granted by the bound resource
/// for an action capability.
///
/// The official Coding resource defaults grant `read`, `write`, and `execute`;
/// destructive filesystem actions therefore consume the workspace `write`
/// grant rather than inventing a separate `delete` permission.
pub fn required_action_resource_operation(
    capability_id: &CapabilityId,
    action_id: &ActionId,
) -> Option<&'static str> {
    match (capability_id.as_ref(), action_id.as_ref()) {
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/read" | "workspace.files/search")
        | (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/status" | "workspace.vcs/diff")
        | (WORKSPACE_ARTIFACTS_MODULE_ID, "workspace.artifacts/read") => Some("read"),
        (WORKSPACE_FILES_MODULE_ID, "workspace.files/write" | "workspace.files/patch" | "workspace.files/delete")
        | (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/stage" | "workspace.vcs/commit" | "workspace.vcs/push")
        | (WORKSPACE_ARTIFACTS_MODULE_ID, "workspace.artifacts/publish")
        | (SSH_MODULE_ID, "ssh/fs.write") => Some("write"),
        (WORKSPACE_PROCESS_MODULE_ID, _) | (SSH_MODULE_ID, "ssh/exec") => Some("execute"),
        (SSH_MODULE_ID, "ssh/sudo") => Some("sudo"),
        (SSH_MODULE_ID, "ssh/fs.read") => Some("read"),
        (BROWSER_MODULE_ID, "browser/observe") => Some("observe"),
        (BROWSER_MODULE_ID, "browser/navigate") => Some("navigate"),
        (BROWSER_MODULE_ID, "browser/act") => Some("act"),
        (BROWSER_MODULE_ID, "browser/render_content") => Some("render_content"),
        (BROWSER_MODULE_ID, "browser/download") => Some("download"),
        (BROWSER_MODULE_ID, "browser/upload") => Some("upload"),
        (BROWSER_MODULE_ID, "browser/evaluate") => Some("evaluate"),
        ("computer.input", "computer.input.invoke") => Some("input"),
        ("computer.launch", "computer.launch.invoke") => Some("launch"),
        _ => None,
    }
}

/// Compatibility query for remaining single-action non-Workspace consumers.
pub fn required_resource_operation(capability_id: &CapabilityId) -> Option<&'static str> {
    let actions = action_ids(capability_id.as_ref());
    (actions.len() == 1)
        .then(|| actions.iter().next())
        .flatten()
        .and_then(|action_id| required_action_resource_operation(capability_id, action_id))
}

/// Validate the authorization-bearing resource projection before an action is
/// handed to an owner.
///
/// This is intentionally reusable by a central host dispatcher. It validates
/// binding identity, principal ownership, required resource cardinality, and
/// the operation grant. It does not resolve a resource or expose an
/// application pool.
pub fn validate_action_resource_bindings(
    capability_id: &CapabilityId,
    action_id: &ActionId,
    principal: &PrincipalRef,
    bindings: &TypedResourceBindings,
) -> Result<(), KernelError> {
    let Some(definition) = definition_for(capability_id.as_ref()) else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 2 capability {}", capability_id.as_ref()),
        });
    };
    if !definition.is_tool() {
        return Err(KernelError::CapabilityExecution {
            reason: format!("{} is not an action capability", capability_id.as_ref()),
        });
    }
    if !declares_action(capability_id, action_id) {
        return Err(KernelError::ActionNotDeclared {
            capability_id: capability_id.clone(),
            action_id: action_id.clone(),
        });
    }

    let declared_resource_kinds = definition
        .resource_kinds
        .iter()
        .map(|kind| ResourceKind::from(*kind))
        .collect::<BTreeSet<_>>();
    let mut binding_ids = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().is_empty() || binding.resource_id.as_ref().is_empty() {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires non-empty binding and resource IDs",
                    capability_id.as_ref()
                ),
            });
        }
        if !binding_ids.insert(binding.binding_id.clone()) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} received duplicate resource binding {}",
                    capability_id.as_ref(),
                    binding.binding_id.as_ref()
                ),
            });
        }
        if binding.owner_id != principal.principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
        if !declared_resource_kinds.contains(&binding.resource_kind) {
            return Err(KernelError::UnexpectedResourceBinding {
                capability_id: capability_id.clone(),
                binding_id: binding.binding_id.clone(),
                resource_kind: binding.resource_kind.as_ref().to_owned(),
            });
        }
    }

    for resource_kind in definition
        .resource_kinds
        .iter()
        .map(|kind| ResourceKind::from(*kind))
    {
        let matching_bindings = bindings
            .iter()
            .filter(|binding| binding.resource_kind == resource_kind)
            .collect::<Vec<_>>();
        if matching_bindings.is_empty() {
            return Err(KernelError::CapabilityResourceNotBound {
                capability_id: capability_id.clone(),
                resource_kind: resource_kind.as_ref().to_owned(),
            });
        }
        if matching_bindings.len() > 1 {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires exactly one {} resource binding",
                    capability_id.as_ref(),
                    resource_kind.as_ref()
                ),
            });
        }
        let missing_operation = required_action_resource_operation(capability_id, action_id)
            .filter(|operation| !matching_bindings[0].operations.contains(*operation));
        if let Some(required_operation) = missing_operation {
            return Err(KernelError::CapabilityResourceNotBound {
                capability_id: capability_id.clone(),
                resource_kind: format!(
                    "{} (operation {})",
                    resource_kind.as_ref(),
                    required_operation
                ),
            });
        }
    }
    Ok(())
}

/// The exact Action identities published by a Module/capability.
pub fn action_ids(capability_id: &str) -> BTreeSet<ActionId> {
    let Some(definition) = definition_for(capability_id).filter(|definition| definition.is_tool())
    else {
        return BTreeSet::new();
    };
    if definition.module_actions.is_empty() {
        BTreeSet::from([ActionId::from(format!("{}.invoke", capability_id))])
    } else {
        definition
            .module_actions
            .iter()
            .map(|action| ActionId::from(action.id))
            .collect()
    }
}

fn declares_action(capability_id: &CapabilityId, action_id: &ActionId) -> bool {
    action_ids(capability_id.as_ref()).contains(action_id)
}

/// Check a Capability against its release-time host target/surface metadata.
///
/// A failed check is represented by the canonical typed Kernel error.  No
/// alternate execution mode is introduced for unavailable Browser or
/// Computer hosts.
pub fn check_platform_availability(
    capability_id: &CapabilityId,
    host_target: &RuntimeTarget,
    host_surface: &str,
) -> Result<(), KernelError> {
    let Some(definition) = definition_for(capability_id.as_ref()) else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 2 capability {}", capability_id.as_ref()),
        });
    };

    if platform_supported(
        definition.platform_scope,
        host_target.as_ref(),
        host_surface,
    ) {
        Ok(())
    } else {
        Err(KernelError::CapabilityUnavailableOnPlatform {
            capability_id: capability_id.clone(),
            target: host_target.as_ref().to_owned(),
            surface: host_surface.to_owned(),
        })
    }
}

/// Return a boolean availability result for callers that need a preflight
/// projection without losing the typed error API above.
pub fn is_available_on_platform(
    capability_id: &str,
    host_target: &str,
    host_surface: &str,
) -> Result<bool, String> {
    let Some(definition) = definition_for(capability_id) else {
        return Err(format!("unknown Wave 2 capability {capability_id}"));
    };
    Ok(platform_supported(
        definition.platform_scope,
        host_target,
        host_surface,
    ))
}

/// Return the canonical error code for headless Browser/Computer failures.
pub fn unavailable_on_platform_code() -> CanonicalErrorCode {
    CanonicalErrorCode::from(CAPABILITY_UNAVAILABLE_ON_PLATFORM)
}

/// Build one package registration from the same factory used by
/// [`registrations`].
pub fn workspace_execution_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[0],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled SSH registration.
pub fn ssh_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[1],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled Browser registration.
pub fn browser_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[2],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled Computer/A11y registration.
pub fn computer_a11y_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[3],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

fn role_id_for_capability(capability_id: &str) -> Option<ExecutionRoleId> {
    if capability_id == BROWSER_MODULE_ID {
        Some(ExecutionRoleId::from(BROWSER_EXECUTION_ROLE_ID))
    } else if capability_id.starts_with("computer.") || capability_id == "a11y.observe" {
        Some(ExecutionRoleId::from(COMPUTER_EXECUTION_ROLE_ID))
    } else {
        None
    }
}

fn role_contracts_for_package(
    package: &PackageDefinition,
    capabilities: &[CapabilityManifest],
) -> Result<Vec<RoleContractManifest>, String> {
    let role_id = match package.id {
        BROWSER_PACKAGE_ID => BROWSER_EXECUTION_ROLE_ID,
        COMPUTER_A11Y_PACKAGE_ID => COMPUTER_EXECUTION_ROLE_ID,
        _ => return Ok(Vec::new()),
    };
    let member_ids = match role_id {
        BROWSER_EXECUTION_ROLE_ID => {
            [(BROWSER_MODULE_ID, RoleMemberRequirement::Required)].as_slice()
        }
        COMPUTER_EXECUTION_ROLE_ID => [
            ("computer.observe", RoleMemberRequirement::Required),
            ("computer.input", RoleMemberRequirement::Required),
            ("computer.launch", RoleMemberRequirement::Optional),
            ("a11y.observe", RoleMemberRequirement::Optional),
        ]
        .as_slice(),
        _ => &[],
    };
    let mut by_id = capabilities
        .iter()
        .map(|capability| (capability.id.as_ref(), capability))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut members = Vec::with_capacity(member_ids.len());
    for (capability_id, requirement) in member_ids {
        let capability = by_id.remove(capability_id).ok_or_else(|| {
            format!(
                "role {} references missing capability {}",
                role_id, capability_id
            )
        })?;
        members.push(RoleMemberContract {
            capability: CapabilityRef {
                id: capability.id.clone(),
                version: capability.version.clone(),
            },
            capability_manifest_digest: digest_payload(capability)
                .map_err(|error| format!("digest role member {capability_id}: {error}"))?,
            requirement: *requirement,
        });
    }
    Ok(vec![RoleContractManifest {
        key: RoleContractKey {
            role_id: ExecutionRoleId::from(role_id),
            contract_version: VersionString::from(if role_id == BROWSER_EXECUTION_ROLE_ID {
                BROWSER_ROLE_CONTRACT_VERSION
            } else {
                CONTRACT_VERSION
            }),
        },
        members,
        serialized_target_resource_kind: match role_id {
            BROWSER_EXECUTION_ROLE_ID => Some(ResourceKind::from(BROWSER_RESOURCE[0])),
            COMPUTER_EXECUTION_ROLE_ID => Some(ResourceKind::from("computer")),
            _ => None,
        },
    }])
}

fn role_providers_for_package(
    package: &PackageDefinition,
    contracts: &[RoleContractManifest],
) -> Result<Vec<RoleProviderContribution>, String> {
    let Some(contract) = contracts.first() else {
        return Ok(Vec::new());
    };
    let supported_platforms = match contract.key.role_id.as_ref() {
        BROWSER_EXECUTION_ROLE_ID => platform_constraints(PlatformScope::BrowserDesktop),
        COMPUTER_EXECUTION_ROLE_ID => platform_constraints(PlatformScope::ComputerDesktop),
        _ => vec![PlatformConstraint::Any],
    };
    let mut members = std::collections::BTreeMap::new();
    for member in &contract.members {
        let required_resource_kinds = definition_for(member.capability.id.as_ref())
            .map(|definition| {
                definition
                    .resource_kinds
                    .iter()
                    .map(|kind| ResourceKind::from(*kind))
                    .collect()
            })
            .unwrap_or_default();
        members.insert(
            member.capability.id.clone(),
            RoleProviderMemberContribution {
                implementation: None,
                supported_platforms: supported_platforms.clone(),
                required_resource_kinds,
            },
        );
    }
    Ok(vec![RoleProviderContribution {
        role: ExactRoleContractRef {
            key: contract.key.clone(),
            contract_digest: digest_payload(contract)
                .map_err(|error| format!("digest {} role contract: {error}", package.id))?,
        },
        display: localized(
            match contract.key.role_id.as_ref() {
                BROWSER_EXECUTION_ROLE_ID => "Browser Use",
                COMPUTER_EXECUTION_ROLE_ID => "Computer Use",
                _ => "System Capability",
            },
            "Bundled first-party execution-role provider.",
        ),
        members,
    }])
}

fn definition_for(capability_id: &str) -> Option<CapabilityDefinition> {
    PACKAGE_DEFINITIONS
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .copied()
        .find(|definition| definition.id == capability_id)
}

fn platform_supported(scope: PlatformScope, host_target: &str, host_surface: &str) -> bool {
    match scope {
        PlatformScope::Any => AGENT_SURFACES.iter().any(|surface| *surface == host_surface),
        PlatformScope::BrowserDesktop => {
            host_surface == "desktop"
                && BROWSER_DESKTOP_HOST_TARGETS
                    .iter()
                    .any(|target| *target == host_target)
        }
        PlatformScope::ComputerDesktop => {
            host_surface == "desktop"
                && COMPUTER_DESKTOP_HOST_TARGETS
                    .iter()
                    .any(|target| *target == host_target)
        }
    }
}

fn platform_constraints(scope: PlatformScope) -> Vec<PlatformConstraint> {
    match scope {
        PlatformScope::Any => vec![PlatformConstraint::Any],
        PlatformScope::BrowserDesktop => target_constraint(BROWSER_DESKTOP_HOST_TARGETS),
        PlatformScope::ComputerDesktop => target_constraint(COMPUTER_DESKTOP_HOST_TARGETS),
    }
}

fn target_constraint(targets: &[&str]) -> Vec<PlatformConstraint> {
    vec![PlatformConstraint::Targets {
        host_targets: targets
            .iter()
            .map(|target| RuntimeTarget::from(*target))
            .collect(),
        host_surfaces: BTreeSet::from(["desktop".to_owned()]),
    }]
}

fn schema_ref(capability_id: &str, role: &str) -> Result<CanonicalSchemaRef, String> {
    let schema = canonical_schema(capability_id, role);
    let digest = digest_payload(&schema)
        .map_err(|error| format!("digest {capability_id} {role} schema: {error}"))?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{capability_id}/{role}@1#{}",
        digest.as_ref()
    )))
}

fn object_schema() -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty_object always returns an object");
    object.insert("type".to_owned(), "object".to_owned().into());
    object.insert("additionalProperties".to_owned(), false.into());
    schema
}

fn open_object_schema() -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty_object always returns an object");
    object.insert("type".to_owned(), "object".to_owned().into());
    object.insert("additionalProperties".to_owned(), true.into());
    schema
}

fn output_schema() -> StrictJsonValue {
    // The owning host service defines the operation result.  The registration
    // only requires an object and does not publish a fabricated result shape.
    open_object_schema()
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

fn localized(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: Default::default(),
        localized_descriptions: Default::default(),
    }
}

fn capability_display(capability_id: &str) -> (&str, &str) {
    match capability_id {
        WORKSPACE_FILES_MODULE_ID => (
            "Workspace Files",
            "Read, search, update, patch, delete, and watch files in a bound workspace.",
        ),
        WORKSPACE_VCS_MODULE_ID => (
            "Version Control",
            "Inspect and explicitly change version-control state in a bound workspace.",
        ),
        WORKSPACE_PROCESS_MODULE_ID => (
            "Workspace Processes",
            "Run and control turn-owned processes inside a bound workspace.",
        ),
        WORKSPACE_ARTIFACTS_MODULE_ID => (
            "Workspace Artifacts",
            "Publish immutable workspace outputs and read them through bounded receipts.",
        ),
        _ => (capability_id, "Bundled Wave 2 coding-extension capability."),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE2_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(WAVE2_CAPABILITY_HOST_PORT_ID, "request")?,
        response_schema: schema_ref(WAVE2_CAPABILITY_HOST_PORT_ID, "response")?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rendered_content_contract_has_only_url_input_and_bounded_html_output() {
        let valid=super::StrictJsonValue(serde_json::json!({"url":"https://example.com/"}));
        assert!(super::validate_module_action_input(super::BROWSER_MODULE_ID,"browser/render_content",&valid).is_ok());
        for value in [serde_json::json!({}),serde_json::json!({"url":"file:///private"}),serde_json::json!({"url":"https://example.com/","chrome_path":"other.exe"})] {
            assert!(super::validate_module_action_input(super::BROWSER_MODULE_ID,"browser/render_content",&super::StrictJsonValue(value)).is_err());
        }
        let output=super::canonical_schema("browser/render_content","output");
        let validator=jsonschema::options().build(&output.0).unwrap();
        assert!(validator.validate(&serde_json::json!({"final_url":"https://example.com/","html":"<p>content</p>","html_truncated":false})).is_ok());
        assert!(validator.validate(&serde_json::json!({"html":"content"})).is_err());
    }
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::task::{Context, Poll, Waker};

    use super::*;
    use serde_json::json;
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest, CompilerEnvironment,
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, Materializer,
        SessionCapabilityState,
    };
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, CapabilityRef,
        CapabilitySelection, DigestHex, PresetRevisionRef, ResourceBindingId, RoleProviderSelection,
        RuntimeProfileKind, UserId, TypedResourceBinding,
    };

    struct StateCaptureHostPort {
        captured: Arc<Mutex<Option<Wave2StateHandle>>>,
    }

    struct AlternateBrowserHandler {
        captured_mount: Arc<Mutex<Option<String>>>,
    }

    impl CapabilityHandler for AlternateBrowserHandler {
        fn invoke<'life0, 'async_trait>(
            &'life0 self,
            context: CapabilityInvocationContext,
            _input: StrictJsonValue,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<StrictJsonValue, KernelError>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: Sync + 'async_trait,
        {
            let captured_mount = Arc::clone(&self.captured_mount);
            Box::pin(async move {
                *captured_mount.lock().expect("alternate provider capture") =
                    Some(context.state.descriptor().mount_id.as_ref().to_owned());
                Ok(empty_object())
            })
        }
    }

    impl Wave2HostPort for StateCaptureHostPort {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let captured = Arc::clone(&self.captured);
            let state = request.context.state;
            Box::pin(std::future::ready({
                *captured.lock().expect("state capture mutex") = Some(state);
                Ok(empty_object())
            }))
        }
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test future must complete without an executor"),
        }
    }

    fn test_state_handle() -> Wave2StateHandle {
        static HANDLE: OnceLock<Wave2StateHandle> = OnceLock::new();
        HANDLE.get_or_init(capture_state_handle).clone()
    }

    fn alternate_browser_registration_with_capture(
        captured_mount: Arc<Mutex<Option<String>>>,
    ) -> PluginRegistration {
        let first_party = browser_registration().expect("first-party Browser registration");
        let provider = first_party.metadata.manifest.payload.contributions.role_providers[0].clone();
        let package = PackageRef {
            id: PackageId::from("fixture.browser-provider"),
            version: VersionString::from(CONTRACT_VERSION),
        };
        let mount_id = PluginMountId::from("fixture-browser-provider");
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::TestFixture,
            source_identity: "fixture.browser-provider".to_owned(),
            source_digest: None,
        };
        let config_schema = object_schema();
        let cancellation_port = host_port("host.plugin.cancel");
        let task_port = host_port("host.plugin.tasks");
        let manifest = PackageManifest {
            schema_version: VersionString::from(CONTRACT_VERSION),
            host_contract_version: VersionString::from(CONTRACT_VERSION),
            package_id: package.id.clone(),
            package_version: package.version.clone(),
            display: localized("Alternate Browser", "Test-only Browser role provider."),
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: config_schema.clone(),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: InProcessEntrypointMetadata {
                entrypoint_profile: "trusted-in-process".to_owned(),
                entrypoint_id: "fixture.browser-provider.entrypoint".to_owned(),
                contract_version: VersionString::from(CONTRACT_VERSION),
            }
            .into(),
            contributions: PackageContributions {
                capabilities: Vec::new(),
                skills: Vec::new(),
                mcp_tools: Vec::new(),
                role_contracts: Vec::new(),
                role_providers: vec![provider.clone()],
            },
        };
        let identity = PluginIdentityDescriptor {
            package: package.clone(),
            mount_id: mount_id.clone(),
        };
        let metadata = PluginRegistrationMetadata {
            manifest: ArtifactEnvelope::new(manifest).expect("alternate Browser manifest"),
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
                    PluginRegistrarOperation::ContributeRoleProvider,
                    PluginRegistrarOperation::BindHostPort,
                ]),
                declared_capability_ids: BTreeSet::new(),
                declared_skill_ids: BTreeSet::new(),
                declared_mcp_tool_keys: BTreeSet::new(),
                declared_role_ids: BTreeSet::from([provider.role.key.role_id.clone()]),
                declared_service_keys: BTreeSet::new(),
                declared_host_ports: BTreeSet::from([
                    cancellation_port.id.clone(),
                    task_port.id.clone(),
                ]),
            },
            context: PluginContextDescriptor {
                identity,
                source,
                validated_config: ValidatedPluginConfig {
                    schema_digest: digest_payload(&config_schema).expect("config digest"),
                    config_revision: 1,
                    value: empty_object(),
                },
                state: PluginStateHandleDescriptor {
                    package_id: package.id,
                    mount_id: mount_id.clone(),
                    methods: PluginStateMethod::REQUIRED.into_iter().collect(),
                },
                declared_services: DeclaredServiceViewDescriptor::default(),
                host_ports: Vec::new(),
                typed_command_ports: Vec::new(),
                domain_outbox_ports: Vec::new(),
                cancellation: CancellationDescriptor {
                    cancellation_port,
                    scope_key: ScopeKey::from("mount:fixture-browser-provider"),
                },
                managed_task_registration: ManagedTaskRegistrationDescriptor {
                    registrar_port: task_port,
                    scope_key: ScopeKey::from("mount:fixture-browser-provider"),
                },
            },
        };
        let mut registration = PluginRegistration::new(metadata);
        for capability_id in provider.members.keys() {
            let Some(definition) = definition_for(capability_id.as_ref()) else {
                continue;
            };
            match definition.kind {
                CapabilityKind::Tool => {
                    registration
                        .add_role_action_handler(
                            provider.role.key.role_id.clone(),
                            capability_id.clone(),
                            Arc::new(AlternateBrowserHandler {
                                captured_mount: Arc::clone(&captured_mount),
                            }),
                        )
                        .expect("alternate Browser role handler");
                }
                _ => {}
            }
        }
        registration
    }

    fn capture_state_handle() -> Wave2StateHandle {
        let captured = Arc::new(Mutex::new(None));
        invoke_workspace_action(
            WORKSPACE_FILES_MODULE_ID,
            "workspace.files/read",
            StrictJsonValue(serde_json::json!({"path":"fixture.txt"})),
            Arc::new(StateCaptureHostPort { captured: Arc::clone(&captured) }),
        )
        .expect("state projection invocation");
        captured
            .lock()
            .expect("state capture mutex")
            .take()
            .expect("host adapter received the state handle")
    }

    fn invoke_workspace_action(
        capability_id: &str,
        action_id: &str,
        input: StrictJsonValue,
        host_port: Arc<dyn Wave2HostPort>,
    ) -> Result<StrictJsonValue, KernelError> {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                registrations_with_host_port(host_port).expect("Wave 2 registrations"),
            )
            .expect("publish Wave 2 registrations");

        let principal = nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave2-state-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("wave2-state-workspace"),
            resource_kind: nomifun_agent_contracts::ResourceKind::from("workspace"),
            resource_id: nomifun_agent_contracts::ResourceId::from("wave2-state-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from([
                required_action_resource_operation(
                    &CapabilityId::from(capability_id),
                    &ActionId::from(action_id),
                )
                    .expect("workspace action requires an operation")
                    .to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let action = ActionId::from(action_id);
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from(CONTRACT_VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from(capability_id),
                    version: VersionString::from(CONTRACT_VERSION),
                },
                action_allowlist: BTreeSet::from([action.clone()]),
            }],

            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Wave 2 state test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            starter_prompts: Vec::new(),
        };
        let contribution_locks = vec![materialized
            .capability(&CapabilityId::from(capability_id))
            .expect("materialized workspace capability")
            .contribution_lock
            .clone()];
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave2-state-test"),
                revision: 1,
                revision_digest: DigestHex::from(""),
            },
            payload,
            contribution_locks,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        revision.reference.revision_digest =
            revision.revision_digest().expect("revision digest");
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("windows-desktop-x64"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave2-state-test".to_owned(),
            },
            CompileRequest {
                plugin_product_capabilities: Vec::new(),
                revision,
                principal: principal.clone(),
                scene: "wave2-state-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave2-state-resolve"),
            },
        )
        .expect("compile selected capability")
        .with_target_resource_bindings(&principal, vec![binding.clone()])
        .expect("bind selected target resource");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        poll_ready(registry.invoke(
            &snapshot,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal,
                agent_session_id: AgentSessionId::from("wave2-state-session"),
                turn_id: OperationId::from("wave2-state-turn"),
                operation_id: OperationId::from("wave2-state-operation"),
                idempotency_key: IdempotencyKey::from("wave2-state-idempotency"),
                correlation_id: CorrelationId::from("wave2-state-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from(capability_id),
                action_id: action,
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
                    "wave2-state-workspace",
                )]),
                state_scope_key: ScopeKey::from("package:wave2-state-test"),
                input,
            },
        ))
    }

    #[test]
    fn kernel_projects_package_and_mount_scoped_state_handle_to_host_adapter() {
        let state = capture_state_handle();
        assert_eq!(
            state.descriptor().package_id.as_ref(),
            WORKSPACE_EXECUTION_PACKAGE_ID
        );
        assert_eq!(
            state.descriptor().mount_id.as_ref(),
            WORKSPACE_EXECUTION_MOUNT_ID
        );
        assert_eq!(
            state.descriptor().methods,
            PluginStateMethod::REQUIRED.into_iter().collect()
        );
    }

    #[test]
    fn registrations_have_unique_inventory_ids_and_exact_handler_coverage() {
        let registrations = registrations().expect("Wave 2 registrations");
        let package_ids = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .package_id
                    .as_ref()
                    .to_owned()
            })
            .collect::<BTreeSet<_>>();
        let actual_capability_ids = registrations
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

        assert_eq!(
            package_ids,
            PACKAGE_IDS
                .iter()
                .map(|package_id| (*package_id).to_owned())
                .collect()
        );
        assert_eq!(actual_capability_ids, capability_ids());
        assert_eq!(actual_capability_ids.len(), ALL_CAPABILITY_IDS.len());
        assert_eq!(
            actual_capability_ids,
            TARGET_CAPABILITY_IDS
                .iter()
                .map(|capability_id| CapabilityId::from(*capability_id))
                .collect()
        );
        assert!(actual_capability_ids
            .iter()
            .all(|capability| !capability.as_ref().starts_with("coding.")));

        for registration in &registrations {
            assert!(
                registration
                    .metadata
                    .manifest
                    .verify()
                    .expect("manifest digest")
            );
            let manifest = &registration.metadata.manifest.payload;
            let action_capabilities = manifest
                .contributions
                .capabilities
                .iter()
                .filter(|capability| !capability.contributions.actions.is_empty())
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>();
            let role_capabilities = manifest
                .contributions
                .role_contracts
                .iter()
                .flat_map(|contract| {
                    contract
                        .members
                        .iter()
                        .map(|member| member.capability.id.clone())
                })
                .collect::<BTreeSet<_>>();
            let ordinary_action_capabilities = action_capabilities
                .difference(&role_capabilities)
                .cloned()
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.handler_ids(), ordinary_action_capabilities);
            let role_action_capabilities = registration
                .role_action_handler_ids()
                .into_iter()
                .map(|(_, capability_id)| capability_id)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                role_action_capabilities,
                action_capabilities
                    .intersection(&role_capabilities)
                    .cloned()
                    .collect()
            );
            for capability in &manifest.contributions.capabilities {
                if capability.kind == CapabilityKind::Tool {
                    assert_eq!(
                        action_ids(capability.id.as_ref()),
                        capability
                            .contributions
                            .actions
                            .iter()
                            .map(|action| action.action_id.clone())
                            .collect()
                    );
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE2_CAPABILITY_HOST_PORT_ID)]
                    );
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(capability.contributions.host_ports.is_empty());
                }
            }
            assert!(manifest
                .contributions
                .capabilities
                .iter()
                .any(|capability| capability.kind == CapabilityKind::Tool));
            assert!(registration
                .metadata
                .context
                .host_ports
                .iter()
                .any(|binding| {
                    binding.port.id == HostPortId::from(WAVE2_CAPABILITY_HOST_PORT_ID)
                }));
            assert!(registration
                .metadata
                .registrar
                .declared_host_ports
                .contains(&HostPortId::from(WAVE2_CAPABILITY_HOST_PORT_ID)));
        }
    }

    #[test]
    fn workspace_package_exposes_four_direct_modules_with_exact_actions() {
        let registration = workspace_execution_registration().unwrap();
        let modules = registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .into_iter()
            .map(|module| (module.id.as_ref().to_owned(), module))
            .collect::<BTreeMap<_, _>>();
        let expected = [
            (WORKSPACE_FILES_MODULE_ID, WORKSPACE_FILES_ACTION_IDS),
            (WORKSPACE_VCS_MODULE_ID, WORKSPACE_VCS_ACTION_IDS),
            (WORKSPACE_PROCESS_MODULE_ID, WORKSPACE_PROCESS_ACTION_IDS),
            (WORKSPACE_ARTIFACTS_MODULE_ID, WORKSPACE_ARTIFACTS_ACTION_IDS),
        ];
        assert_eq!(modules.len(), expected.len());
        for (module_id, expected_actions) in expected {
            let module = &modules[module_id];
            assert_eq!(
                module.authoring_policy().unwrap(),
                nomifun_agent_contracts::CapabilityAuthoringPolicy::Direct
            );
            assert_eq!(
                module.contributions.actions.iter()
                    .map(|action| action.action_id.as_ref())
                    .collect::<BTreeSet<_>>(),
                expected_actions.iter().copied().collect::<BTreeSet<_>>()
            );
            assert_eq!(
                module.contributions.resource_kinds.len(),
                1,
                "each workspace Module requires one typed owner resource"
            );
            let mut presentation_only_kind = module.clone();
            presentation_only_kind.kind = CapabilityKind::ContextContributor;
            assert_eq!(
                presentation_only_kind.authoring_policy().unwrap(),
                CapabilityAuthoringPolicy::Direct,
                "explicit Module authoring policy must not be inferred from display kind"
            );
            assert_eq!(
                presentation_only_kind.contributions.actions,
                module.contributions.actions,
                "changing display kind must not rewrite Action contributions"
            );
        }
    }

    #[test]
    fn workspace_modules_publish_exact_resolvable_action_schemas() {
        let capabilities = registrations()
            .expect("Wave 2 registrations")
            .into_iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
            })
            .map(|capability| (capability.id.as_ref().to_owned(), capability))
            .collect::<std::collections::BTreeMap<_, _>>();
        for (capability_id, action_id) in [
            (WORKSPACE_FILES_MODULE_ID, "workspace.files/delete"),
            (WORKSPACE_VCS_MODULE_ID, "workspace.vcs/push"),
            (WORKSPACE_PROCESS_MODULE_ID, "workspace.process/start"),
            (WORKSPACE_ARTIFACTS_MODULE_ID, "workspace.artifacts/publish"),
        ] {
            let action = capabilities[capability_id]
                .contributions
                .actions
                .iter()
                .find(|action| action.action_id.as_ref() == action_id)
                .expect("Module action");
            let schema = resolve_action_schema(capability_id, &action.input_schema)
                .expect("manifest input ref resolves from its canonical source");
            assert_eq!(schema.0["additionalProperties"], serde_json::json!(false));
            assert!(schema.0["required"].as_array().is_some_and(|fields| !fields.is_empty()));
            assert!(schema.0["properties"].get("owner").is_none());
            assert!(schema.0["properties"].get("session_id").is_none());
            assert!(schema.0["properties"].get("workspace_root").is_none());
            assert_eq!(
                schema_ref(action_id, "input").unwrap(),
                action.input_schema,
                "manifest and resolver must share one canonical schema reference"
            );
            assert!(
                action
                    .input_schema
                    .as_ref()
                    .ends_with(nomifun_agent_contracts::digest_payload(&schema).unwrap().as_ref())
            );
        }

        assert!(validate_module_action_input(
            WORKSPACE_FILES_MODULE_ID,
            "workspace.files/delete",
            &StrictJsonValue(serde_json::json!({"path": "src/lib.rs"})),
        ).is_ok());
        assert!(validate_module_action_input(
            WORKSPACE_FILES_MODULE_ID,
            "workspace.files/delete",
            &StrictJsonValue(serde_json::json!({
                "path": "src/lib.rs",
                "workspace_root": "C:/spoof"
            })),
        ).is_err());
        assert!(validate_module_action_input(
            WORKSPACE_PROCESS_MODULE_ID,
            "workspace.process/start",
            &StrictJsonValue(serde_json::json!({
                "operation": "start",
                "command": "git"
            })),
        ).is_err(), "the Action ID, not payload state, selects the process operation");
        assert!(validate_module_action_input(
            WORKSPACE_VCS_MODULE_ID,
            "workspace.vcs/push",
            &StrictJsonValue(serde_json::json!({
                "remote": "origin",
                "refspec": "HEAD:refs/heads/main",
                "force": true
            })),
        ).is_err());
    }

    #[test]
    fn strict_workspace_inputs_are_rejected_before_kernel_owner_dispatch() {
        for (capability_id, action_id, invalid, valid) in [
            (
                WORKSPACE_FILES_MODULE_ID,
                "workspace.files/delete",
                json!({"path": " \t"}),
                json!({"path": "src/lib.rs"}),
            ),
            (
                WORKSPACE_ARTIFACTS_MODULE_ID,
                "workspace.artifacts/publish",
                json!({"path": ""}),
                json!({"path": "target/result.txt"}),
            ),
            (
                WORKSPACE_VCS_MODULE_ID,
                "workspace.vcs/push",
                json!({"remote": "origin", "refspec": "HEAD:refs/heads/main", "force": true}),
                json!({"remote": "origin", "refspec": "HEAD:refs/heads/main", "force": false}),
            ),
        ] {
            let captured = Arc::new(Mutex::new(None));
            let host: Arc<dyn Wave2HostPort> = Arc::new(StateCaptureHostPort {
                captured: Arc::clone(&captured),
            });
            for input in [json!(null), json!([]), invalid] {
                let error = invoke_workspace_action(
                    capability_id,
                    action_id,
                    StrictJsonValue(input),
                    Arc::clone(&host),
                )
                .expect_err("invalid input must not reach the owner");
                assert_eq!(error.canonical_code().as_ref(), INVALID_PAYLOAD);
                assert!(captured.lock().unwrap().is_none());
            }
            invoke_workspace_action(capability_id, action_id, StrictJsonValue(valid), host)
                .expect("valid input with the declared resource grant reaches the owner");
            assert!(captured.lock().unwrap().is_some());
        }
    }

    #[test]
    fn workspace_files_changed_event_is_module_derived_without_a_fake_watch_action() {
        assert_eq!(
            required_resource_kinds(WORKSPACE_FILES_MODULE_ID),
            Some(BTreeSet::from([ResourceKind::from("workspace")]))
        );
        assert!(!action_ids(WORKSPACE_FILES_MODULE_ID)
            .contains(&ActionId::from("workspace.files/watch")));
        let manifest = workspace_execution_registration()
            .unwrap()
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .into_iter()
            .find(|capability| capability.id.as_ref() == WORKSPACE_FILES_MODULE_ID)
            .unwrap();
        assert_eq!(manifest.contributions.event_schema_refs.len(), 1);
        let batch = WorkspaceFilesChangedBatch::new(
            vec![WorkspaceFileChangedEvent {
                path: "src/lib.rs".into(),
                kind: WorkspaceFileChangeKind::Modified,
            }],
            0,
        );
        batch.validate().unwrap();
        let payload = serde_json::to_value(&batch).unwrap();
        let schema = canonical_schema(WORKSPACE_FILES_MODULE_ID, "event");
        jsonschema::options()
            .build(&schema.0)
            .unwrap()
            .validate(&payload)
            .unwrap();
        let mut drifted = payload;
        drifted["events"][0]["kind"] = json!("modify");
        assert!(jsonschema::options()
            .build(&schema.0)
            .unwrap()
            .validate(&drifted)
            .is_err());
    }

    #[test]
    fn registrations_materialize_and_publish_through_the_kernel_contract() {
        let registrations = registrations().expect("Wave 2 registrations");
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(CONTRACT_VERSION),
            &registrations,
            1,
        )
        .expect("Wave 2 metadata materializes");
        assert_eq!(materialized.packages.len(), 4);
        assert_eq!(materialized.capabilities.len(), 10);
        assert_eq!(materialized.role_contracts.len(), 2);
        assert_eq!(materialized.role_providers.len(), 2);
        let browser_role = materialized.role_contract(&ExecutionRoleId::from(BROWSER_EXECUTION_ROLE_ID))
            .expect("Browser Role v2");
        assert_eq!(browser_role.manifest.key.contract_version.as_ref(), BROWSER_ROLE_CONTRACT_VERSION);
        assert_eq!(browser_role.manifest.members.iter().map(|member| member.capability.id.as_ref()).collect::<BTreeSet<_>>(), BTreeSet::from([BROWSER_MODULE_ID]));
        for member in &browser_role.manifest.members {
            assert_eq!(
                required_resource_kinds(member.capability.id.as_ref()),
                Some(BTreeSet::from([ResourceKind::from("browser")]))
            );
        }
        assert_eq!(
            required_resource_kinds(WORKSPACE_PROCESS_MODULE_ID),
            Some(BTreeSet::from([ResourceKind::from("process_session")]))
        );
        for retired in ["process.session", "terminal.pty", "workspace.bind"] {
            assert!(required_resource_kinds(retired).is_none());
        }

        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        registry
            .replace_all(registrations)
            .expect("all action handlers are declared");
    }

    #[test]
    fn explicit_browser_provider_lock_dispatches_once_without_fallback() {
        let first_party = browser_registration().expect("first-party Browser registration");
        let captured_mount = Arc::new(Mutex::new(None));
        let alternate =
            alternate_browser_registration_with_capture(
                Arc::clone(&captured_mount),
            );
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable_with_test_fixtures(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(vec![first_party, alternate])
            .expect("publish two Browser providers");
        let role_id = ExecutionRoleId::from(BROWSER_EXECUTION_ROLE_ID);
        let contract = materialized
            .role_contract(&role_id)
            .expect("Browser role contract");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "browser-provider-owner".to_owned(),
        };
        let revision = |overrides: BTreeMap<ExecutionRoleId, RoleProviderSelection>| {
            let payload = AgentPresetRevisionPayload {
                context_order: Vec::new(),
                middleware_order: Vec::new(),
                schema_version: VersionString::from(CONTRACT_VERSION),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
                enabled_capabilities: vec![CapabilitySelection {
                    capability: CapabilityRef {
                        id: CapabilityId::from(BROWSER_MODULE_ID),
                        version: VersionString::from(CONTRACT_VERSION),
                    },
                    action_allowlist: BTreeSet::from([
                        ActionId::from("browser/observe"),
                        ActionId::from("browser/navigate"),
                    ]),
                }],

                skill_bindings: Vec::new(),
                system_role_provider_overrides: overrides,
                persona: "Browser provider fixture".to_owned(),
                instructions: "Navigate with the selected Browser provider.".to_owned(),
                starter_prompts: Vec::new(),
            };
            let contribution_locks = payload
                .enabled_capabilities
                .iter()
                .map(|selection| {
                    materialized
                        .capability(&selection.capability.id)
                        .expect("selected Browser capability is materialized")
                        .contribution_lock
                        .clone()
                })
                .collect();
            let mut revision = AgentPresetRevision {
                reference: PresetRevisionRef {
                    preset_id: AgentPresetId::from("browser-provider-fixture"),
                    revision: 1,
                    revision_digest: DigestHex::from(""),
                },
                payload,
                contribution_locks,
                created_by: UserId::from(principal.principal_id.clone()),
                created_at_ms: 1,
                reason: None,
            };
            revision.reference.revision_digest =
                revision.revision_digest().expect("revision digest");
            revision
        };
        let environment = CompilerEnvironment {
            resolver_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("runtime"),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("target"),
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: "browser-provider-test".to_owned(),
        };
        assert!(matches!(
            AgentPresetCompiler::compile(
                &materialized,
                &environment,
                CompileRequest {
                    plugin_product_capabilities: Vec::new(),
                    revision: revision(BTreeMap::new()),
                    principal: principal.clone(),
                    scene: "test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("browser-provider-no-selection"),
                },
            ),
            Err(KernelError::RoleProviderNotBound { .. })
        ));

        let selected = RoleProviderSelection {
            role: ExactRoleContractRef {
                key: contract.manifest.key.clone(),
                contract_digest: contract.contract_digest.clone(),
            },
            provider_mount_id: PluginMountId::from("fixture-browser-provider"),
        };
        let browser_binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser-binding"),
            resource_kind: ResourceKind::from("browser"),
            resource_id: "browser-resource".into(),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["observe".to_owned(), "navigate".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let compiled = AgentPresetCompiler::compile(
            &materialized,
            &environment,
            CompileRequest {
                plugin_product_capabilities: Vec::new(),
                revision: revision(BTreeMap::from([(role_id.clone(), selected)])),
                principal: principal.clone(),
                scene: "test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("browser-provider-selected"),
            },
        )
        .expect("compile selected alternate Browser provider")
        .with_target_resource_bindings(&principal, vec![browser_binding.clone()])
        .expect("bind alternate Browser target resource");
        assert_eq!(
            compiled
                .role_provider(&role_id)
                .expect("frozen Browser provider")
                .provider
                .mount_id
                .as_ref(),
            "fixture-browser-provider"
        );
        let active = SessionCapabilityState::new(&compiled)
            .snapshot()
            .expect("initial active set");
        poll_ready(registry.invoke(
            &compiled,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal.clone(),
                agent_session_id: AgentSessionId::from("browser-provider-session"),
                turn_id: OperationId::from("browser-provider-turn"),
                operation_id: OperationId::from("browser-provider-invoke"),
                idempotency_key: IdempotencyKey::from("browser-provider-invoke"),
                correlation_id: CorrelationId::from("browser-provider-invoke"),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from(BROWSER_MODULE_ID),
                action_id: ActionId::from("browser/navigate"),
                resource_binding_ids: BTreeSet::from([browser_binding.binding_id]),
                state_scope_key: ScopeKey::from("session:browser-provider"),
                input: StrictJsonValue(json!({"url":"https://example.test/"})),
            },
        ))
        .expect("alternate Browser invocation");
        assert_eq!(
            captured_mount
                .lock()
                .expect("alternate provider capture")
                .as_deref(),
            Some("fixture-browser-provider")
        );
        assert_eq!(
            compiled.content().required_resource_kinds,
            BTreeSet::from([ResourceKind::from("browser")])
        );
    }

    #[test]
    fn every_action_capability_maps_to_a_typed_host_operation() {
        assert!(matches!(
            typed_operation_for(
                &CapabilityId::from(WORKSPACE_FILES_MODULE_ID),
                &ActionId::from("workspace.files/read"),
                empty_object(),
            ),
            Ok(Wave2TypedCapabilityOperation::WorkspaceFileRead { .. })
        ));
        assert!(matches!(
            typed_operation_for(
                &CapabilityId::from(WORKSPACE_PROCESS_MODULE_ID),
                &ActionId::from("workspace.process/start"),
                empty_object(),
            ),
            Ok(Wave2TypedCapabilityOperation::WorkspaceProcessStart { .. })
        ));
        assert!(matches!(
            typed_operation_for(
                &CapabilityId::from(WORKSPACE_ARTIFACTS_MODULE_ID),
                &ActionId::from("workspace.artifacts/publish"),
                empty_object(),
            ),
            Ok(Wave2TypedCapabilityOperation::WorkspaceArtifactPublish { .. })
        ));

        for definition in PACKAGE_DEFINITIONS
            .iter()
            .flat_map(|package| package.capabilities.iter())
            .filter(|definition| definition.is_tool())
        {
            let capability_id = CapabilityId::from(definition.id);
            for action_id in action_ids(definition.id) {
                let typed = typed_operation_for(
                    &capability_id,
                    &action_id,
                    empty_object(),
                )
                .expect("Module Actions must have an exact typed operation");
                assert_eq!(typed.capability_id(), definition.id);
                assert_eq!(typed.action_id(), action_id.as_ref());
                assert!(
                    operation_for(&capability_id, &action_id, empty_object()).is_ok(),
                    "{}/{} must have a host operation",
                    definition.id,
                    action_id.as_ref(),
                );
            }
        }
    }

    #[test]
    fn non_action_capabilities_cannot_enter_the_host_dispatch_contract() {
        let error = typed_operation_for(
            &CapabilityId::from("unknown.resource"),
            &ActionId::from("unknown.resource.invoke"),
            empty_object(),
        )
            .expect_err("resource providers must not become action operations");
        assert!(error.to_string().contains("does not expose action"));
    }

    #[test]
    fn host_request_rejects_wrong_family_and_missing_or_unauthorized_bindings() {
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let binding = nomifun_agent_contracts::TypedResourceBinding {
            binding_id: "workspace-binding".into(),
            resource_kind: "workspace".into(),
            resource_id: "workspace-resource".into(),
            owner_id: "owner".to_owned(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        };
        let context = Wave2HostContext {
            principal: principal.clone(),
            agent_session_id: AgentSessionId::from("session"),
            turn_id: OperationId::from("turn"),
            operation_id: OperationId::from("operation"),
            idempotency_key: IdempotencyKey::from("idempotency"),
            correlation_id: CorrelationId::from("correlation"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "digest".into(),
            },
            registry_generation: 7,
            capability_id: CapabilityId::from(WORKSPACE_FILES_MODULE_ID),
            action_id: ActionId::from("workspace.files/read"),
            role_provider: None,
            state: test_state_handle(),
            resource_bindings: vec![binding.clone()],
        };
        let wrong_family = match (Wave2HostRequest {
            context: context.clone(),
            operation: Wave2CapabilityOperation::Ssh {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("workspace.files/read cannot be routed through the SSH family"),
            Err(error) => error,
        };
        assert_eq!(wrong_family.code, "ACTION_OPERATION_MISMATCH");

        let missing_binding = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: Vec::new(),
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive an unbound action"),
            Err(error) => error,
        };
        assert_eq!(missing_binding.code, PRESET_RESOURCE_NOT_BOUND);

        let wrong_owner = match (Wave2HostRequest {
            context: Wave2HostContext {
                principal: PrincipalRef {
                    principal_id: "different-owner".to_owned(),
                    ..principal
                },
                resource_bindings: vec![binding.clone()],
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive another principal's binding"),
            Err(error) => error,
        };
        assert_eq!(wrong_owner.code, RESOURCE_OWNER_MISMATCH);

        let unexpected_binding = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: vec![
                    binding.clone(),
                    nomifun_agent_contracts::TypedResourceBinding {
                        binding_id: "ssh-binding".into(),
                        resource_kind: "ssh_host".into(),
                        resource_id: "ssh-resource".into(),
                        owner_id: "owner".to_owned(),
                        operations: BTreeSet::from(["read".to_owned()]),
                        connection_config_ref: None,
                        typed_parameters: Default::default(),
                    },
                ],
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
         .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive undeclared resource bindings"),
            Err(error) => error,
        };
        assert_eq!(unexpected_binding.code, PRESET_RESOURCE_NOT_BOUND);
        assert!(unexpected_binding.message.contains("ssh_host"));

        let missing_grant = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: vec![nomifun_agent_contracts::TypedResourceBinding {
                    operations: BTreeSet::new(),
                    ..binding
                }],
                ..context
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive a binding without read grant"),
            Err(error) => error,
        };
        assert_eq!(missing_grant.code, PRESET_RESOURCE_NOT_BOUND);
    }

    #[test]
    fn module_actions_require_their_exact_resource_operation() {
        let capability_id = CapabilityId::from(WORKSPACE_FILES_MODULE_ID);
        let action_id = ActionId::from("workspace.files/read");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("workspace-binding"),
            resource_kind: ResourceKind::from("workspace"),
            resource_id: "workspace-1".into(),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        };

        validate_action_resource_bindings(
            &capability_id,
            &action_id,
            &principal,
            &vec![binding],
        )
        .expect("Module Action must consume its exact resource operation");
    }

    #[test]
    fn composed_typed_adapter_receives_exact_operation_and_authorization_projection() {
        let seen = Arc::new(Mutex::new(None));
        let seen_by_adapter = Arc::clone(&seen);
        let adapter = typed_operation_adapter(
            |operation| matches!(operation, Wave2TypedCapabilityOperation::WorkspaceFileRead { .. }),
            move |request| {
                let seen = Arc::clone(&seen_by_adapter);
                std::future::ready({
                    let is_exact = matches!(
                        request.operation,
                        Wave2TypedCapabilityOperation::WorkspaceFileRead { .. }
                    );
                    let authorization = (
                        request.context.principal.principal_id,
                        request.context.registry_generation,
                        request.context.resource_bindings[0].resource_id.clone(),
                    );
                    *seen.lock().expect("test adapter mutex") =
                        Some((is_exact, authorization));
                    Ok(empty_object())
                })
            },
        );
        let dispatcher = Wave2HostPortDispatcher::new(vec![adapter]);
        let request = Wave2HostRequest {
            context: Wave2HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "owner".to_owned(),
                },
                agent_session_id: AgentSessionId::from("session"),
                turn_id: OperationId::from("turn"),
                operation_id: OperationId::from("operation"),
                idempotency_key: IdempotencyKey::from("idempotency"),
                correlation_id: CorrelationId::from("correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 11,
                capability_id: CapabilityId::from(WORKSPACE_FILES_MODULE_ID),
                action_id: ActionId::from("workspace.files/read"),
                role_provider: None,
                state: test_state_handle(),
                resource_bindings: vec![nomifun_agent_contracts::TypedResourceBinding {
                    binding_id: "binding".into(),
                    resource_kind: "workspace".into(),
                    resource_id: "resource".into(),
                    owner_id: "owner".to_owned(),
                    operations: BTreeSet::from(["read".to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: Default::default(),
                }],
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        };
        poll_ready(dispatcher.invoke(request.clone())).expect("configured adapter succeeds");
        assert_eq!(
            *seen.lock().expect("test adapter mutex"),
            Some((
                true,
                (
                    "owner".to_owned(),
                    11,
                    nomifun_agent_contracts::ResourceId::from("resource"),
                )
            ))
        );

        let empty_dispatcher = Wave2HostPortDispatcher::empty();
        let mut unsupported = request;
        unsupported.context.action_id = ActionId::from("workspace.files/write");
        unsupported.context.resource_bindings[0].operations = BTreeSet::from(["write".to_owned()]);
        let unavailable = poll_ready(empty_dispatcher.invoke(unsupported))
            .expect_err("unsupported owner-backed action must fail closed");
        assert_eq!(unavailable.code, CAPABILITY_UNAVAILABLE);
    }

    #[test]
    fn unconfigured_action_host_returns_a_typed_unavailable_error() {
        let host_port = unconfigured_host_port();
        let future = host_port.invoke(Wave2HostRequest {
                context: Wave2HostContext {
                    principal: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: "wave2-test-owner".to_owned(),
                    },
                    agent_session_id: AgentSessionId::from("wave2-test-session"),
                    turn_id: OperationId::from("wave2-test-turn"),
                    operation_id: OperationId::from("wave2-test-operation"),
                    idempotency_key: IdempotencyKey::from("wave2-test-idempotency"),
                    correlation_id: CorrelationId::from("wave2-test-correlation"),
                    resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: "snapshot".into(),
                        snapshot_digest: "digest".into(),
                    },
                    registry_generation: 1,
                    capability_id: CapabilityId::from(WORKSPACE_FILES_MODULE_ID),
                    action_id: ActionId::from("workspace.files/read"),
                    role_provider: None,
                    state: test_state_handle(),
                    resource_bindings: Vec::new(),
                },
                operation: Wave2CapabilityOperation::WorkspaceExecution {
                    input: empty_object(),
                },
            });
        let result = poll_ready(future).expect_err("unconfigured Wave 2 actions must fail closed");
        assert_eq!(result.code, CAPABILITY_UNAVAILABLE);
        assert_eq!(
            result.message,
            "no production host adapter is bound for workspace.files"
        );

        let kernel_error = wave2_host_error_to_kernel(Wave2HostPortError::new(
            "VCS_OWNER_REJECTED",
            "VCS owner rejected the request",
        ));
        let failure = kernel_error
            .capability_execution_failure()
            .expect("Wave 2 host errors cross the Kernel as a typed failure");
        assert_eq!(failure.code.as_ref(), "VCS_OWNER_REJECTED");
        assert_eq!(failure.message, "VCS owner rejected the request");

        let invalid = wave2_input_error_to_kernel(KernelError::CapabilityExecution {
            reason: "request body was invalid".to_owned(),
        });
        assert_eq!(invalid.canonical_code().as_ref(), INVALID_PAYLOAD);
    }

    #[test]
    fn browser_and_computer_platform_metadata_fail_closed_on_headless_hosts() {
        let registrations = registrations().expect("Wave 2 registrations");
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(CONTRACT_VERSION),
            &registrations,
            1,
        )
        .expect("Wave 2 metadata materializes");

        for capability_id in BROWSER_CAPABILITY_IDS {
            let capability = materialized
                .capability(&CapabilityId::from(*capability_id))
                .expect("Browser capability");
            assert_eq!(
                capability.manifest.supported_platforms,
                vec![PlatformConstraint::Any]
            );
            assert_eq!(
                capability.manifest.supported_surfaces,
                BTreeSet::from([
                    "authoring:direct".to_owned(),
                    "consumer:agent".to_owned(),
                    "desktop".to_owned(),
                ])
            );
            assert!(check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "desktop",
            )
            .is_ok());
            let error = check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "headless",
            )
            .expect_err("headless Browser must be unavailable");
            assert_eq!(error.canonical_code(), unavailable_on_platform_code());
        }

        for capability_id in COMPUTER_A11Y_CAPABILITY_IDS {
            let capability = materialized
                .capability(&CapabilityId::from(*capability_id))
                .expect("Computer/A11y capability");
            assert_eq!(
                capability.manifest.supported_platforms,
                vec![PlatformConstraint::Any]
            );
            assert_eq!(
                capability.manifest.supported_surfaces,
                BTreeSet::from([
                    "consumer:agent".to_owned(),
                    "desktop".to_owned(),
                ])
            );
            for (target, surface) in [
                ("x86_64-unknown-linux-gnu", "desktop"),
                ("x86_64-unknown-linux-gnu", "headless"),
            ] {
                let error = check_platform_availability(
                    &CapabilityId::from(*capability_id),
                    &RuntimeTarget::from(target),
                    surface,
                )
                .expect_err("unsupported Computer host must be unavailable");
                assert_eq!(error.canonical_code(), unavailable_on_platform_code());
            }
            assert!(check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-pc-windows-msvc"),
                "desktop",
            )
            .is_ok());
        }

        for (role_id, expected_targets) in [
            (
                BROWSER_EXECUTION_ROLE_ID,
                BROWSER_DESKTOP_HOST_TARGETS,
            ),
            (
                COMPUTER_EXECUTION_ROLE_ID,
                COMPUTER_DESKTOP_HOST_TARGETS,
            ),
        ] {
            let provider = materialized
                .role_provider(
                    &ExecutionRoleId::from(role_id),
                    &PluginMountId::from(if role_id == BROWSER_EXECUTION_ROLE_ID {
                        BROWSER_MOUNT_ID
                    } else {
                        COMPUTER_A11Y_MOUNT_ID
                    }),
                )
                .expect("first-party role provider");
            for member in provider.contribution.members.values() {
                assert!(member.supported_platforms.iter().all(|constraint| {
                    matches!(
                        constraint,
                        PlatformConstraint::Targets {
                            host_targets,
                            host_surfaces,
                        } if host_targets
                            == &expected_targets
                                .iter()
                                .map(|target| RuntimeTarget::from(*target))
                                .collect()
                            && host_surfaces == &BTreeSet::from(["desktop".to_owned()])
                    )
                }));
            }
        }
    }
}
